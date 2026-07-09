use std::{
    fmt::{self, Debug, Formatter},
    fs::DirBuilder,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use anyhow::Result;
use async_trait::async_trait;
use rusqlite::{Connection, params};
use syneroym_core::{
    config::SubstrateConfig, local_registry::SubstrateEndpoint, storage::EndpointStorage,
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

            // Schema versioning
            let version: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
            if version == 0 {
                // Basic schema creation for endpoints
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
                conn.execute("PRAGMA user_version = 1", [])?;
            }

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
}
