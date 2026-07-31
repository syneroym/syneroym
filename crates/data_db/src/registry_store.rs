use std::{
    fmt::{self, Debug, Formatter},
    fs::DirBuilder,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
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
            // M04A Slice B7a: service ownership. Separate table, not a
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
            // A2: which app instance and logical name a deployed service
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
            // A4: what a deploy said this service *is*, and how to probe it.
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
                    created_at        INTEGER NOT NULL
                );",
                [],
            )?;
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
            // M05A A5a: who manages an app instance on this substrate
            // (ADR-0021 §4). Replaces A2's `app_instance_owners` outright --
            // pre-release, so no `ALTER TABLE`/version ladder (see AGENTS.md);
            // the old table simply stops being created. `owner_did` keeps
            // A2's first-write-wins takeover guard; `supervisor_did`/
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

    async fn load_all_deploy_facts(&self) -> Result<Vec<(String, String, Option<String>)>> {
        let conn_arc = self.conn.clone();
        task::spawn_blocking(move || -> Result<Vec<(String, String, Option<String>)>> {
            let conn = lock_db(&conn_arc)?;
            let mut stmt = conn.prepare(
                "SELECT service_id, service_type, health_check_json FROM service_deploy_facts",
            )?;
            let mut facts = Vec::new();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                facts.push((row.get(0)?, row.get(1)?, row.get(2)?));
            }
            Ok(facts)
        })
        .await?
    }

    async fn save_deploy_facts(
        &self,
        service_id: &str,
        service_type: &str,
        health_check_json: Option<&str>,
    ) -> Result<()> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();
        let stype = service_type.to_string();
        let check = health_check_json.map(str::to_string);
        let created_at: i64 =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute(
                "INSERT INTO service_deploy_facts (service_id, service_type, health_check_json, \
                 created_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(service_id) DO UPDATE SET
                    service_type = excluded.service_type,
                    health_check_json = excluded.health_check_json,
                    created_at = excluded.created_at",
                params![sid, stype, check, created_at],
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
            // `(app_instance_id, dependency_name)` (D-A2-10, last-write-
            // wins) replay the same way on every restart instead of
            // depending on SQLite's unspecified row order.
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
            .map_err(anyhow::Error::from)
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
mod tests {
    use tempfile::{TempDir, tempdir};

    use super::*;

    async fn make_store() -> (SqliteEndpointStorage, TempDir) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let store = SqliteEndpointStorage::new(path).await.unwrap();
        (store, dir)
    }

    /// A4-15: every other deploy-facts test writes through `MockStorage` or
    /// a live single-process deploy -- nothing proves the round trip through
    /// a real `SqliteEndpointStorage` file and back out, which is the fact
    /// D-A4-7/D-A4-17 rest on (`instance_phase`'s recorded-type lookup and
    /// `readyz`'s repaired guess). If this path regressed, every service on
    /// a rebooted node would silently fall to `Unknown("no service type
    /// recorded")` and `readyz` would quietly stop inspecting containers --
    /// a healthy-looking failure the suite would otherwise never catch.
    #[tokio::test]
    async fn deploy_facts_survive_a_real_reopen_of_the_same_database_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("registry.db");

        {
            let store = SqliteEndpointStorage::new(&path).await.unwrap();
            store
                .save_deploy_facts(
                    "svc-1",
                    "tcp",
                    Some(r#"{"tcp-connect":{"interface-name":"main","timeout-ms":2000}}"#),
                )
                .await
                .unwrap();
        }

        // A genuinely fresh connection to the same file, not the same
        // `SqliteEndpointStorage` instance.
        let reopened = SqliteEndpointStorage::new(&path).await.unwrap();
        let facts = reopened.load_all_deploy_facts().await.unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].0, "svc-1");
        assert_eq!(facts[0].1, "tcp");
        assert!(facts[0].2.as_deref().unwrap().contains("tcp-connect"));
    }

    #[tokio::test]
    async fn test_save_and_load_wasm() {
        let (store, _dir) = make_store().await;
        let ep = SubstrateEndpoint::WasmChannel { service_id: "app-123".to_string() };
        store.save("app-123", "greet", &ep).await.unwrap();

        let all = store.load_all().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, "app-123");
        assert_eq!(all[0].1, "greet");
        assert!(
            matches!(&all[0].2, SubstrateEndpoint::WasmChannel { service_id } if service_id == "app-123")
        );
    }

    #[tokio::test]
    async fn test_save_and_load_tcp() {
        let (store, _dir) = make_store().await;
        let ep = SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 8080 };
        store.save("app-tcp", "api", &ep).await.unwrap();

        let all = store.load_all().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, "app-tcp");
        assert_eq!(all[0].1, "api");
        assert!(
            matches!(&all[0].2, SubstrateEndpoint::TcpHostPort { host, port } if host == "127.0.0.1" && *port == 8080)
        );
    }

    #[tokio::test]
    async fn test_save_and_load_podman() {
        let (store, _dir) = make_store().await;
        let ep =
            SubstrateEndpoint::PodmanSocket { socket_path: "/var/run/podman.sock".to_string() };
        store.save("app-podman", "socket", &ep).await.unwrap();

        let all = store.load_all().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, "app-podman");
        assert_eq!(all[0].1, "socket");
        assert!(
            matches!(&all[0].2, SubstrateEndpoint::PodmanSocket { socket_path } if socket_path == "/var/run/podman.sock")
        );
    }

    #[tokio::test]
    async fn test_save_upserts() {
        let (store, _dir) = make_store().await;
        let ep1 = SubstrateEndpoint::WasmChannel { service_id: "v1".to_string() };
        let ep2 = SubstrateEndpoint::WasmChannel { service_id: "v2".to_string() };
        store.save("svc", "iface", &ep1).await.unwrap();
        store.save("svc", "iface", &ep2).await.unwrap();

        let all = store.load_all().await.unwrap();
        assert_eq!(all.len(), 1);
        assert!(
            matches!(&all[0].2, SubstrateEndpoint::WasmChannel { service_id } if service_id == "v2")
        );
    }

    #[tokio::test]
    async fn test_remove() {
        let (store, _dir) = make_store().await;
        let ep = SubstrateEndpoint::NativeHostChannel { service_id: "sub-1".to_string() };
        store.save("sub-1", "orchestrator", &ep).await.unwrap();
        store.remove("sub-1", "orchestrator").await.unwrap();
        assert!(store.load_all().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_load_invalid_endpoints() {
        let (store, _dir) = make_store().await;

        // Save a valid one first
        let ep = SubstrateEndpoint::WasmChannel { service_id: "valid-1".to_string() };
        store.save("valid-1", "iface", &ep).await.unwrap();

        // Directly insert invalid rows to mock corrupted DB data
        let conn_arc = store.conn.clone();
        task::spawn_blocking(move || {
            let conn = conn_arc.lock().unwrap();

            // Invalid endpoint type key
            conn.execute(
                "INSERT INTO local_endpoints (service_id, interface_name, endpoint_type, \
                 endpoint_data)
                 VALUES ('invalid-type', 'iface', 'unknown_type', 'some_data')",
                [],
            )
            .unwrap();

            // Invalid TCP data format (no colon)
            conn.execute(
                "INSERT INTO local_endpoints (service_id, interface_name, endpoint_type, \
                 endpoint_data)
                 VALUES ('invalid-tcp-no-colon', 'iface', 'tcp', '127.0.0.1')",
                [],
            )
            .unwrap();

            // Invalid TCP data format (non-integer port)
            conn.execute(
                "INSERT INTO local_endpoints (service_id, interface_name, endpoint_type, \
                 endpoint_data)
                 VALUES ('invalid-tcp-bad-port', 'iface', 'tcp', '127.0.0.1:abc')",
                [],
            )
            .unwrap();
        })
        .await
        .unwrap();

        // load_all should skip the invalid ones, warning about them, and still return
        // the valid one
        let all = store.load_all().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, "valid-1");
        assert!(
            matches!(&all[0].2, SubstrateEndpoint::WasmChannel { service_id } if service_id == "valid-1")
        );
    }

    /// M04A Slice B7a: a freshly created `endpoints.db` gets both tables in
    /// the same `version == 0` migration block -- `service_owners` is usable
    /// immediately, no separate migration step.
    #[tokio::test]
    async fn test_fresh_db_gets_service_owners_table() {
        let (store, _dir) = make_store().await;
        store.save_owner("svc-1", "did:key:zOwner").await.unwrap();
        let owners = store.load_all_owners().await.unwrap();
        assert_eq!(owners, vec![("svc-1".to_string(), "did:key:zOwner".to_string())]);
    }

    #[tokio::test]
    async fn test_save_owner_upserts() {
        let (store, _dir) = make_store().await;
        store.save_owner("svc-1", "did:key:zAlice").await.unwrap();
        store.save_owner("svc-1", "did:key:zBob").await.unwrap();

        let owners = store.load_all_owners().await.unwrap();
        assert_eq!(owners, vec![("svc-1".to_string(), "did:key:zBob".to_string())]);
    }

    #[tokio::test]
    async fn test_remove_owner() {
        let (store, _dir) = make_store().await;
        store.save_owner("svc-1", "did:key:zOwner").await.unwrap();
        store.remove_owner("svc-1").await.unwrap();
        assert!(store.load_all_owners().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_remove_owner_is_idempotent() {
        let (store, _dir) = make_store().await;
        store.remove_owner("never-owned").await.unwrap();
    }

    #[tokio::test]
    async fn a_fresh_db_gets_the_certificate_table() {
        let (store, _dir) = make_store().await;
        store.save_cert("svc-1", r#"{"fake":"cert"}"#).await.unwrap();
        let certs = store.load_all_certs().await.unwrap();
        assert_eq!(certs, vec![("svc-1".to_string(), r#"{"fake":"cert"}"#.to_string())]);
    }

    #[tokio::test]
    async fn saving_a_certificate_upserts() {
        let (store, _dir) = make_store().await;
        store.save_cert("svc-1", r#"{"v":1}"#).await.unwrap();
        store.save_cert("svc-1", r#"{"v":2}"#).await.unwrap();

        let certs = store.load_all_certs().await.unwrap();
        assert_eq!(certs, vec![("svc-1".to_string(), r#"{"v":2}"#.to_string())]);
    }

    #[tokio::test]
    async fn removing_a_service_removes_its_certificate() {
        let (store, _dir) = make_store().await;
        store.save_cert("svc-1", r#"{"fake":"cert"}"#).await.unwrap();
        store.remove_cert("svc-1").await.unwrap();
        assert!(store.load_all_certs().await.unwrap().is_empty());
    }

    /// An existing database, already at `PRAGMA user_version == 1` from
    /// before the certificate table existed, must still gain it on the next
    /// open -- this is the regression a `version < 2` migration gate would
    /// have reintroduced. Built with a raw `Connection` rather than
    /// `SqliteEndpointStorage::new`: that constructor already creates the
    /// certificate table unconditionally, so opening through it here would
    /// make this test pass identically whether or not the gate it exists to
    /// catch was reintroduced -- it has to reproduce the pre-existing,
    /// version-1, certificate-table-less file directly.
    #[tokio::test]
    async fn an_existing_database_gains_the_certificate_table_on_open() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");

        {
            let conn = Connection::open(&path).unwrap();
            conn.execute(
                "CREATE TABLE local_endpoints (
                    service_id TEXT NOT NULL,
                    interface_name TEXT NOT NULL,
                    endpoint_type TEXT NOT NULL,
                    endpoint_data TEXT NOT NULL,
                    PRIMARY KEY (service_id, interface_name)
                );",
                [],
            )
            .unwrap();
            conn.execute(
                "CREATE TABLE service_owners (
                    service_id TEXT PRIMARY KEY,
                    owner_did  TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );",
                [],
            )
            .unwrap();
            conn.execute("PRAGMA user_version = 1", []).unwrap();
        }

        // Reopen through the real constructor -- schema creation must run
        // unconditionally and add the certificate table to this
        // pre-existing, version-1 file.
        let store = SqliteEndpointStorage::new(&path).await.unwrap();
        store.save_cert("svc-1", r#"{"fake":"cert"}"#).await.unwrap();
        let certs = store.load_all_certs().await.unwrap();
        assert_eq!(certs, vec![("svc-1".to_string(), r#"{"fake":"cert"}"#.to_string())]);
    }

    /// A2's own version of the same regression: a database that predates
    /// `service_app_context`/`service_bindings` must still gain both tables
    /// on the next open, for the identical reason `an_existing_database_
    /// gains_the_certificate_table_on_open` exists one table over.
    #[tokio::test]
    async fn an_existing_database_gains_the_app_context_and_binding_tables_on_open() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");

        {
            let conn = Connection::open(&path).unwrap();
            conn.execute(
                "CREATE TABLE local_endpoints (
                    service_id TEXT NOT NULL,
                    interface_name TEXT NOT NULL,
                    endpoint_type TEXT NOT NULL,
                    endpoint_data TEXT NOT NULL,
                    PRIMARY KEY (service_id, interface_name)
                );",
                [],
            )
            .unwrap();
            conn.execute("PRAGMA user_version = 1", []).unwrap();
        }

        let store = SqliteEndpointStorage::new(&path).await.unwrap();
        store.save_app_context("svc-1", "app-1", "backend").await.unwrap();
        store.save_binding("svc-1", "app-1", "backend", r#"{"fake":"entry"}"#).await.unwrap();
        assert_eq!(
            store.load_all_app_contexts().await.unwrap(),
            vec![("svc-1".to_string(), "app-1".to_string(), "backend".to_string())]
        );
        assert_eq!(
            store.load_all_bindings().await.unwrap(),
            vec![(
                "svc-1".to_string(),
                "app-1".to_string(),
                "backend".to_string(),
                r#"{"fake":"entry"}"#.to_string()
            )]
        );
    }

    #[tokio::test]
    async fn saving_an_app_context_upserts() {
        let (store, _dir) = make_store().await;
        store.save_app_context("svc-1", "app-1", "backend").await.unwrap();
        store.save_app_context("svc-1", "app-2", "backend-v2").await.unwrap();

        let contexts = store.load_all_app_contexts().await.unwrap();
        assert_eq!(
            contexts,
            vec![("svc-1".to_string(), "app-2".to_string(), "backend-v2".to_string())]
        );
    }

    #[tokio::test]
    async fn saving_a_binding_upserts_in_place() {
        let (store, _dir) = make_store().await;
        store.save_binding("svc-1", "app-1", "backend", r#"{"v":1}"#).await.unwrap();
        store.save_binding("svc-1", "app-1", "backend", r#"{"v":2}"#).await.unwrap();

        let bindings = store.load_all_bindings().await.unwrap();
        assert_eq!(
            bindings,
            vec![(
                "svc-1".to_string(),
                "app-1".to_string(),
                "backend".to_string(),
                r#"{"v":2}"#.to_string()
            )]
        );
    }

    /// Finding 05 (post-review fix): `load_all_bindings`'s replay consumer
    /// (`substrate::runtime::replay_persisted_bindings`) discards
    /// `service_id` and keys purely on `(app_instance_id, dependency_name)`
    /// -- last-write-wins (D-A2-10) means a conflict between two services'
    /// rows for the same instance/name depends entirely on iteration order.
    /// Inserted deliberately out of both id order and insertion order, so a
    /// `SELECT` with no `ORDER BY` (SQLite's row order is otherwise
    /// unspecified) would have a real chance of returning them unsorted.
    #[tokio::test]
    async fn loading_all_bindings_is_ordered_by_service_id_then_dependency_name() {
        let (store, _dir) = make_store().await;
        store.save_binding("svc-b", "app-1", "y-dep", r#"{"v":1}"#).await.unwrap();
        store.save_binding("svc-a", "app-1", "z-dep", r#"{"v":1}"#).await.unwrap();
        store.save_binding("svc-a", "app-1", "x-dep", r#"{"v":1}"#).await.unwrap();
        store.save_binding("svc-b", "app-1", "a-dep", r#"{"v":1}"#).await.unwrap();

        let bindings = store.load_all_bindings().await.unwrap();
        let ids: Vec<(&str, &str)> = bindings
            .iter()
            .map(|(sid, _instance, dep, _entry)| (sid.as_str(), dep.as_str()))
            .collect();
        assert_eq!(
            ids,
            vec![("svc-a", "x-dep"), ("svc-a", "z-dep"), ("svc-b", "a-dep"), ("svc-b", "y-dep")],
            "replay must be reproducible across restarts regardless of insertion order or \
             SQLite's own unspecified row order"
        );
    }

    #[tokio::test]
    async fn removing_an_app_context_removes_its_binding_rows_too() {
        let (store, _dir) = make_store().await;
        store.save_app_context("svc-1", "app-1", "backend").await.unwrap();
        store.save_binding("svc-1", "app-1", "backend-dep", r#"{"fake":"entry"}"#).await.unwrap();

        store.remove_app_context("svc-1").await.unwrap();

        assert!(store.load_all_app_contexts().await.unwrap().is_empty());
        assert!(store.load_all_bindings().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn removing_an_app_context_is_idempotent() {
        let (store, _dir) = make_store().await;
        store.remove_app_context("never-deployed").await.unwrap();
    }
}
