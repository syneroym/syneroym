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
    mailbox::HostedMailbox,
    storage::{AppInstanceManagement, DeployFacts, EndpointStorage},
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
            init_schema(&conn)?;
            Ok(conn)
        })
        .await??;

        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }
}

fn init_schema(conn: &Connection) -> Result<()> {
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
    conn.execute(
        "CREATE TABLE IF NOT EXISTS service_owners (
            service_id TEXT PRIMARY KEY,
            owner_did  TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS service_mailboxes (
            service_id   TEXT PRIMARY KEY,
            mailbox      TEXT NOT NULL,
            max_ttl_secs INTEGER NOT NULL,
            methods      TEXT NOT NULL DEFAULT '[]',
            created_at   INTEGER NOT NULL
        );",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS service_instance_certs (
            service_id  TEXT PRIMARY KEY,
            certificate TEXT NOT NULL,
            created_at  INTEGER NOT NULL
        );",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS service_app_context (
            service_id      TEXT PRIMARY KEY,
            app_instance_id TEXT NOT NULL,
            service_name    TEXT NOT NULL,
            created_at      INTEGER NOT NULL
        );",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS service_deploy_facts (
            service_id        TEXT PRIMARY KEY,
            service_type      TEXT NOT NULL,
            health_check_json TEXT,
            created_at        INTEGER NOT NULL,
            manifest_hash     TEXT,
            visibility        TEXT,
            wake_json         TEXT
        );",
        [],
    )?;
    migrate_deploy_facts_columns(conn)?;
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
    Ok(())
}

fn migrate_deploy_facts_columns(conn: &Connection) -> Result<()> {
    for col in ["manifest_hash TEXT", "visibility TEXT", "wake_json TEXT"] {
        let sql = format!("ALTER TABLE service_deploy_facts ADD COLUMN {col}");
        if let Err(err) = conn.execute(&sql, [])
            && !err.to_string().contains("duplicate column name")
        {
            return Err(err.into());
        }
    }
    Ok(())
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

    async fn load_all_mailboxes(&self) -> Result<Vec<(String, HostedMailbox)>> {
        let conn_arc = self.conn.clone();
        task::spawn_blocking(move || -> Result<Vec<(String, HostedMailbox)>> {
            let conn = lock_db(&conn_arc)?;
            let mut stmt = conn.prepare(
                "SELECT service_id, mailbox, max_ttl_secs, methods FROM service_mailboxes",
            )?;
            let mut mailboxes = Vec::new();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let mailbox = HostedMailbox {
                    mailbox: row.get(1)?,
                    max_ttl_secs: row.get::<_, i64>(2)? as u64,
                    methods: serde_json::from_str(&row.get::<_, String>(3)?)?,
                };
                mailboxes.push((row.get(0)?, mailbox));
            }
            Ok(mailboxes)
        })
        .await?
    }

    async fn save_mailbox(&self, service_id: &str, mailbox: &HostedMailbox) -> Result<()> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();
        let mailbox = mailbox.clone();
        let methods = serde_json::to_string(&mailbox.methods)?;
        let created_at: i64 =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute(
                "INSERT INTO service_mailboxes
                    (service_id, mailbox, max_ttl_secs, methods, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(service_id) DO UPDATE SET
                    mailbox = excluded.mailbox,
                    max_ttl_secs = excluded.max_ttl_secs,
                    methods = excluded.methods,
                    created_at = excluded.created_at",
                params![sid, mailbox.mailbox, mailbox.max_ttl_secs as i64, methods, created_at],
            )?;
            Ok(())
        })
        .await?
    }

    async fn remove_mailbox(&self, service_id: &str) -> Result<()> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute("DELETE FROM service_mailboxes WHERE service_id = ?1", params![sid])?;
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

    async fn load_all_deploy_facts(&self) -> Result<Vec<(String, DeployFacts)>> {
        let conn_arc = self.conn.clone();
        task::spawn_blocking(move || -> Result<Vec<(String, DeployFacts)>> {
            let conn = lock_db(&conn_arc)?;
            let mut stmt = conn.prepare(
                "SELECT service_id, service_type, health_check_json, manifest_hash, visibility, \
                 wake_json FROM service_deploy_facts",
            )?;
            let mut facts = Vec::new();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let service_id: String = row.get(0)?;
                let deploy_facts = DeployFacts {
                    service_type: row.get(1)?,
                    health_check_json: row.get(2)?,
                    manifest_hash: row.get(3)?,
                    visibility: row.get(4)?,
                    wake_json: row.get(5)?,
                };
                facts.push((service_id, deploy_facts));
            }
            Ok(facts)
        })
        .await?
    }

    async fn save_deploy_facts(&self, service_id: &str, facts: &DeployFacts) -> Result<()> {
        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();
        let facts = facts.clone();
        let created_at: i64 =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);

        task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute(
                "INSERT INTO service_deploy_facts (service_id, service_type, health_check_json, \
                 manifest_hash, visibility, wake_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(service_id) DO UPDATE SET
                    service_type = excluded.service_type,
                    health_check_json = excluded.health_check_json,
                    manifest_hash = excluded.manifest_hash,
                    visibility = excluded.visibility,
                    wake_json = excluded.wake_json,
                    created_at = excluded.created_at",
                params![
                    sid,
                    facts.service_type,
                    facts.health_check_json,
                    facts.manifest_hash,
                    facts.visibility,
                    facts.wake_json,
                    created_at
                ],
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
