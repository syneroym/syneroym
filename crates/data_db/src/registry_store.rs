use std::{
    fmt::{self, Debug, Formatter},
    fs::DirBuilder,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Error, Result};
use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, params};
use syneroym_core::{
    config::SubstrateConfig,
    local_registry::SubstrateEndpoint,
    storage::{AppInstanceManagement, EndpointStorage},
};
use tokio::task;

pub async fn init_store(config: &SubstrateConfig) -> Result<Arc<dyn EndpointStorage>> {
    let db_path = &config.storage.db_dir;
    if !db_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder.create(db_path)?;
        }
        #[cfg(not(unix))]
        {
            std::fs::create_dir_all(db_path)?;
        }
    }
    let db_file = db_path.join("endpoints.db");
    Ok(Arc::new(SqliteEndpointStorage::new(&db_file).await?))
}

pub struct SqliteEndpointStorage {
    conn: Arc<Mutex<Connection>>,
}

impl Debug for SqliteEndpointStorage {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqliteEndpointStorage").field("conn", &"rusqlite::Connection").finish()
    }
}

impl SqliteEndpointStorage {
    /// Create a new `SqliteEndpointStorage` with the given DB path.
    pub async fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let path = db_path.as_ref().to_owned();
        let conn = task::spawn_blocking(move || -> Result<Connection> {
            let conn = Connection::open(path)?;

            // Schema creation runs unconditionally on every open, not gated
            // on `PRAGMA user_version`. It did use to gate on `version == 0`,
            // but every dev and test database created since `service_owners`
            // was added is already at version 1 -- a table added inside that
            // gate would never be created on any of them. `CREATE TABLE IF
            // NOT EXISTS` is already idempotent, so the gate bought nothing
            // it doesn't already provide, and this makes every future
            // in-place schema addition correct by default (pre-release: no
            // compat shims, no version ladders).
            conn.execute(
                "CREATE TABLE IF NOT EXISTS local_endpoints (
                    service_id TEXT NOT NULL,
                    interface_name TEXT NOT NULL,
                    endpoint_type TEXT NOT NULL,
                    endpoint_data TEXT NOT NULL,
                    PRIMARY KEY (service_id, interface_name)
                );",
                [],
            )?;
            // Service ownership. Separate table, not a
            // column on local_endpoints -- ownership is per service, and
            // local_endpoints is keyed (service_id, interface_name), so a
            // column would duplicate the owner across every interface and
            // admit disagreement between rows.
            conn.execute(
                "CREATE TABLE IF NOT EXISTS service_owners (
                    service_id TEXT PRIMARY KEY,
                    owner_did  TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );",
                [],
            )?;
            // Instance certificates (ADR-0020 §1): the DelegationCertificate
            // binding this substrate's derived instance key to the member
            // master a service_id names. Same shape as service_owners --
            // one row per service, upserted on renewal.
            conn.execute(
                "CREATE TABLE IF NOT EXISTS service_instance_certs (
                    service_id  TEXT PRIMARY KEY,
                    certificate TEXT NOT NULL,
                    created_at  INTEGER NOT NULL
                );",
                [],
            )?;
            // Which app instance and logical name a deployed service
            // belongs to, and its resolved dependency bindings. Same
            // unconditional-creation reasoning as the tables above.
            conn.execute(
                "CREATE TABLE IF NOT EXISTS service_app_context (
                    service_id      TEXT PRIMARY KEY,
                    app_instance_id TEXT NOT NULL,
                    service_name    TEXT NOT NULL,
                    created_at      INTEGER NOT NULL
                );",
                [],
            )?;
            // What a deploy said this service *is*, and how to probe it.
            //
            // The type is not derivable after the fact: a container and a
            // TCP service both register `SubstrateEndpoint::TcpHostPort`
            // (orchestration.rs) and `PodmanSocket` is never registered at
            // all, so the endpoint variant cannot tell them apart -- which is
            // why `readyz` had to guess. One row per service, upserted on
            // redeploy, deleted on undeploy, mirroring service_instance_certs.
            conn.execute(
                "CREATE TABLE IF NOT EXISTS service_deploy_facts (
                    service_id        TEXT PRIMARY KEY,
                    service_type      TEXT NOT NULL,
                    health_check_json TEXT,
                    created_at        INTEGER NOT NULL,
                    manifest_hash     TEXT,
                    visibility        TEXT
                );",
                [],
            )?;
            // `manifest_hash`: the canonical content hash of what was
            // actually installed, written only on full deploy success --
            // the dedup key, distinct from the epoch guard and the
            // generation gate (ADR-0021 §3).
            //
            // Unlike every table above, `service_deploy_facts` predates this
            // column, so `CREATE TABLE IF NOT EXISTS` is a no-op here --
            // it never adds a column to a table that already exists. This
            // `ALTER TABLE` is the one idempotent way to get the column onto
            // a database that opened before it existed; it is not the
            // version-ladder AGENTS.md rules out, since there is no schema
            // version to track and nothing else in this method depends on
            // it. "duplicate column name" is the expected outcome once this
            // has already run once, on every open after the first.
            if let Err(err) =
                conn.execute("ALTER TABLE service_deploy_facts ADD COLUMN manifest_hash TEXT", [])
                && !err.to_string().contains("duplicate column name")
            {
                return Err(err.into());
            }
            if let Err(err) =
                conn.execute("ALTER TABLE service_deploy_facts ADD COLUMN visibility TEXT", [])
                && !err.to_string().contains("duplicate column name")
            {
                return Err(err.into());
            }
            conn.execute(
                "CREATE TABLE IF NOT EXISTS service_bindings (
                    service_id      TEXT NOT NULL,
                    app_instance_id TEXT NOT NULL,
                    dependency_name TEXT NOT NULL,
                    entry_json      TEXT NOT NULL,
                    created_at      INTEGER NOT NULL,
                    PRIMARY KEY (service_id, dependency_name)
                );",
                [],
            )?;
            // Who manages an app instance on this substrate (ADR-0021 §4).
            // Replaces the old `app_instance_owners` table outright --
            // pre-release, so no `ALTER TABLE`/version ladder (see AGENTS.md);
            // the old table simply stops being created. `owner_did` keeps
            // the first-write-wins takeover guard; `supervisor_did`/
            // `generation` are new: `None`/`0` means "unmanaged", which is
            // what every operator-driven `roymctl app deploy` sends and what
            // an un-adopted instance accepts.
            conn.execute(
                "CREATE TABLE IF NOT EXISTS app_instance_management (
                    app_instance_id TEXT PRIMARY KEY,
                    owner_did       TEXT NOT NULL,
                    supervisor_did  TEXT,
                    generation      INTEGER NOT NULL DEFAULT 0,
                    created_at      INTEGER NOT NULL
                );",
                [],
            )?;
            conn.execute("PRAGMA user_version = 1", [])?;

            Ok(conn)
        })
        .await??;

        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }
}

fn lock_db(conn: &Arc<Mutex<Connection>>) -> Result<MutexGuard<'_, Connection>> {
    conn.lock().map_err(|e| anyhow::anyhow!("Database connection mutex poisoned: {e}"))
}

#[async_trait]
impl EndpointStorage for SqliteEndpointStorage {
    async fn load_all(&self) -> Result<Vec<(String, String, SubstrateEndpoint)>> {
        let conn_arc = self.conn.clone();
        task::spawn_blocking(move || -> Result<Vec<(String, String, SubstrateEndpoint)>> {
            let conn = lock_db(&conn_arc)?;
            let mut stmt = conn.prepare(
                "SELECT service_id, interface_name, endpoint_type, endpoint_data FROM \
                 local_endpoints",
            )?;

            let mut endpoints = Vec::new();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let sid: String = row.get(0)?;
                let iname: String = row.get(1)?;
                let key: String = row.get(2)?;
                let data: String = row.get(3)?;
                match SubstrateEndpoint::try_from((key.as_str(), data.clone())) {
                    Ok(ep) => {
                        endpoints.push((sid, iname, ep));
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse endpoint for service_id: {}, interface: {}, key: {}, \
                             data: {}: {:?}",
                            sid,
                            iname,
                            key,
                            data,
                            e
                        );
                    }
                }
            }

            Ok(endpoints)
        })
        .await?
    }

    async fn save(
        &self,
        service_id: &str,
        interface_name: &str,
        endpoint: &SubstrateEndpoint,
    ) -> Result<()> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();
        let iname = interface_name.to_string();
        let e_type = endpoint.storage_key().to_string();
        let e_data = endpoint.storage_data();

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute(
                "INSERT INTO local_endpoints (service_id, interface_name, endpoint_type, \
                 endpoint_data)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(service_id, interface_name) DO UPDATE SET
                    endpoint_type = excluded.endpoint_type,
                    endpoint_data = excluded.endpoint_data",
                params![sid, iname, e_type, e_data],
            )?;
            Ok(())
        })
        .await?
    }

    async fn remove(&self, service_id: &str, interface_name: &str) -> Result<()> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();
        let iname = interface_name.to_string();

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute(
                "DELETE FROM local_endpoints WHERE service_id = ?1 AND interface_name = ?2",
                params![sid, iname],
            )?;
            Ok(())
        })
        .await?
    }

    async fn load_all_owners(&self) -> Result<Vec<(String, String)>> {
        let conn_arc = self.conn.clone();
        task::spawn_blocking(move || -> Result<Vec<(String, String)>> {
            let conn = lock_db(&conn_arc)?;
            let mut stmt = conn.prepare("SELECT service_id, owner_did FROM service_owners")?;
            let mut owners = Vec::new();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                owners.push((row.get(0)?, row.get(1)?));
            }
            Ok(owners)
        })
        .await?
    }

    async fn save_owner(&self, service_id: &str, owner_did: &str) -> Result<()> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();
        let owner = owner_did.to_string();
        let created_at: i64 =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute(
                "INSERT INTO service_owners (service_id, owner_did, created_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(service_id) DO UPDATE SET
                    owner_did = excluded.owner_did,
                    created_at = excluded.created_at",
                params![sid, owner, created_at],
            )?;
            Ok(())
        })
        .await?
    }

    async fn remove_owner(&self, service_id: &str) -> Result<()> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute("DELETE FROM service_owners WHERE service_id = ?1", params![sid])?;
            Ok(())
        })
        .await?
    }

    async fn load_all_certs(&self) -> Result<Vec<(String, String)>> {
        let conn_arc = self.conn.clone();
        task::spawn_blocking(move || -> Result<Vec<(String, String)>> {
            let conn = lock_db(&conn_arc)?;
            let mut stmt =
                conn.prepare("SELECT service_id, certificate FROM service_instance_certs")?;
            let mut certs = Vec::new();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                certs.push((row.get(0)?, row.get(1)?));
            }
            Ok(certs)
        })
        .await?
    }

    async fn save_cert(&self, service_id: &str, certificate_json: &str) -> Result<()> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();
        let cert = certificate_json.to_string();
        let created_at: i64 =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute(
                "INSERT INTO service_instance_certs (service_id, certificate, created_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(service_id) DO UPDATE SET
                    certificate = excluded.certificate,
                    created_at = excluded.created_at",
                params![sid, cert, created_at],
            )?;
            Ok(())
        })
        .await?
    }

    async fn remove_cert(&self, service_id: &str) -> Result<()> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute("DELETE FROM service_instance_certs WHERE service_id = ?1", params![sid])?;
            Ok(())
        })
        .await?
    }

    async fn load_all_deploy_facts(
        &self,
    ) -> Result<Vec<(String, String, Option<String>, Option<String>, Option<String>)>> {
        let conn_arc = self.conn.clone();
        task::spawn_blocking(
            move || -> Result<Vec<(String, String, Option<String>, Option<String>, Option<String>)>> {
                let conn = lock_db(&conn_arc)?;
                let mut stmt = conn.prepare(
                    "SELECT service_id, service_type, health_check_json, manifest_hash, visibility FROM \
                     service_deploy_facts",
                )?;
                let mut facts = Vec::new();
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    facts.push((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?));
                }
                Ok(facts)
            },
        )
        .await?
    }

    async fn save_deploy_facts(
        &self,
        service_id: &str,
        service_type: &str,
        health_check_json: Option<&str>,
        manifest_hash: Option<&str>,
        visibility: Option<&str>,
    ) -> Result<()> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();
        let stype = service_type.to_string();
        let check = health_check_json.map(str::to_string);
        let hash = manifest_hash.map(str::to_string);
        let vis = visibility.map(str::to_string);
        let created_at: i64 =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute(
                "INSERT INTO service_deploy_facts (service_id, service_type, health_check_json, \
                 manifest_hash, visibility, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(service_id) DO UPDATE SET
                    service_type = excluded.service_type,
                    health_check_json = excluded.health_check_json,
                    manifest_hash = excluded.manifest_hash,
                    visibility = excluded.visibility,
                    created_at = excluded.created_at",
                params![sid, stype, check, hash, vis, created_at],
            )?;
            Ok(())
        })
        .await?
    }

    async fn remove_deploy_facts(&self, service_id: &str) -> Result<()> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute("DELETE FROM service_deploy_facts WHERE service_id = ?1", params![sid])?;
            Ok(())
        })
        .await?
    }

    async fn load_all_app_contexts(&self) -> Result<Vec<(String, String, String)>> {
        let conn_arc = self.conn.clone();
        task::spawn_blocking(move || -> Result<Vec<(String, String, String)>> {
            let conn = lock_db(&conn_arc)?;
            let mut stmt = conn.prepare(
                "SELECT service_id, app_instance_id, service_name FROM service_app_context",
            )?;
            let mut contexts = Vec::new();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                contexts.push((row.get(0)?, row.get(1)?, row.get(2)?));
            }
            Ok(contexts)
        })
        .await?
    }

    async fn save_app_context(
        &self,
        service_id: &str,
        app_instance_id: &str,
        service_name: &str,
    ) -> Result<()> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();
        let instance = app_instance_id.to_string();
        let name = service_name.to_string();
        let created_at: i64 =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute(
                "INSERT INTO service_app_context (service_id, app_instance_id, service_name, \
                 created_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(service_id) DO UPDATE SET
                    app_instance_id = excluded.app_instance_id,
                    service_name = excluded.service_name,
                    created_at = excluded.created_at",
                params![sid, instance, name, created_at],
            )?;
            Ok(())
        })
        .await?
    }

    async fn remove_app_context(&self, service_id: &str) -> Result<()> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute("DELETE FROM service_app_context WHERE service_id = ?1", params![sid])?;
            conn.execute("DELETE FROM service_bindings WHERE service_id = ?1", params![sid])?;
            Ok(())
        })
        .await?
    }

    async fn load_all_bindings(&self) -> Result<Vec<(String, String, String, String)>> {
        let conn_arc = self.conn.clone();
        task::spawn_blocking(move || -> Result<Vec<(String, String, String, String)>> {
            let conn = lock_db(&conn_arc)?;
            // `ORDER BY` makes a multi-writer conflict on the same
            // `(app_instance_id, dependency_name)` (last-write-wins) replay
            // the same way on every restart instead of depending on
            // SQLite's unspecified row order.
            let mut stmt = conn.prepare(
                "SELECT service_id, app_instance_id, dependency_name, entry_json FROM \
                 service_bindings ORDER BY service_id, dependency_name",
            )?;
            let mut bindings = Vec::new();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                bindings.push((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?));
            }
            Ok(bindings)
        })
        .await?
    }

    async fn save_binding(
        &self,
        service_id: &str,
        app_instance_id: &str,
        dependency_name: &str,
        topology_entry_json: &str,
    ) -> Result<()> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();
        let instance = app_instance_id.to_string();
        let name = dependency_name.to_string();
        let entry = topology_entry_json.to_string();
        let created_at: i64 =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute(
                "INSERT INTO service_bindings (service_id, app_instance_id, dependency_name, \
                 entry_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(service_id, dependency_name) DO UPDATE SET
                    app_instance_id = excluded.app_instance_id,
                    entry_json = excluded.entry_json,
                    created_at = excluded.created_at",
                params![sid, instance, name, entry, created_at],
            )?;
            Ok(())
        })
        .await?
    }

    async fn load_binding(
        &self,
        service_id: &str,
        dependency_name: &str,
    ) -> Result<Option<String>> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();
        let name = dependency_name.to_string();

        task::spawn_blocking(move || -> Result<Option<String>> {
            let conn = lock_db(&conn_arc)?;
            conn.query_row(
                "SELECT entry_json FROM service_bindings WHERE service_id = ?1 AND \
                 dependency_name = ?2",
                params![sid, name],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::from)
        })
        .await?
    }

    async fn load_bindings_for(&self, service_id: &str) -> Result<Vec<(String, String)>> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();

        task::spawn_blocking(move || -> Result<Vec<(String, String)>> {
            let conn = lock_db(&conn_arc)?;
            let mut stmt = conn.prepare(
                "SELECT dependency_name, entry_json FROM service_bindings WHERE service_id = ?1 \
                 ORDER BY dependency_name",
            )?;
            let mut bindings = Vec::new();
            let mut rows = stmt.query(params![sid])?;
            while let Some(row) = rows.next()? {
                bindings.push((row.get(0)?, row.get(1)?));
            }
            Ok(bindings)
        })
        .await?
    }

    async fn load_all_app_instance_management(
        &self,
    ) -> Result<Vec<(String, AppInstanceManagement)>> {
        let conn_arc = self.conn.clone();
        task::spawn_blocking(move || -> Result<Vec<(String, AppInstanceManagement)>> {
            let conn = lock_db(&conn_arc)?;
            let mut stmt = conn.prepare(
                "SELECT app_instance_id, owner_did, supervisor_did, generation FROM \
                 app_instance_management",
            )?;
            let mut rows_out = Vec::new();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let app_instance_id: String = row.get(0)?;
                let owner_did: String = row.get(1)?;
                let supervisor_did: Option<String> = row.get(2)?;
                let generation: i64 = row.get(3)?;
                rows_out.push((
                    app_instance_id,
                    AppInstanceManagement {
                        owner_did,
                        supervisor_did,
                        generation: generation as u64,
                    },
                ));
            }
            Ok(rows_out)
        })
        .await?
    }

    async fn save_app_instance_management(
        &self,
        app_instance_id: &str,
        management: &AppInstanceManagement,
    ) -> Result<()> {
        let conn_arc = self.conn.clone();
        let instance = app_instance_id.to_string();
        let owner = management.owner_did.clone();
        let supervisor = management.supervisor_did.clone();
        let generation = management.generation as i64;
        let created_at: i64 =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute(
                "INSERT INTO app_instance_management (app_instance_id, owner_did, supervisor_did, \
                 generation, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(app_instance_id) DO UPDATE SET
                    owner_did = excluded.owner_did,
                    supervisor_did = excluded.supervisor_did,
                    generation = excluded.generation,
                    created_at = excluded.created_at",
                params![instance, owner, supervisor, generation, created_at],
            )?;
            Ok(())
        })
        .await?
    }

    async fn remove_app_instance_management(&self, app_instance_id: &str) -> Result<()> {
        let conn_arc = self.conn.clone();
        let instance = app_instance_id.to_string();

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute(
                "DELETE FROM app_instance_management WHERE app_instance_id = ?1",
                params![instance],
            )?;
            Ok(())
        })
        .await?
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
