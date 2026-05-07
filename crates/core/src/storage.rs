use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::{config::SubstrateConfig, registry::SubstrateEndpoint};
use anyhow::Result;

use async_trait::async_trait;
use rusqlite::params;

/// A trait abstracting stable storage for the EndpointRegistry.
#[async_trait]
pub trait EndpointStorage: Send + Sync {
    /// Load all endpoints from stable storage. Returns a vector of (service_id, interface_name, endpoint).
    async fn load_all(&self) -> Result<Vec<(String, String, SubstrateEndpoint)>>;

    /// Save an endpoint into stable storage.
    async fn save(
        &self,
        service_id: &str,
        interface_name: &str,
        endpoint: &SubstrateEndpoint,
    ) -> Result<()>;

    /// Remove an endpoint from stable storage.
    async fn remove(&self, service_id: &str, interface_name: &str) -> Result<()>;
}

pub async fn init_store(config: &SubstrateConfig) -> Result<Arc<dyn EndpointStorage>> {
    let db_path = &config.storage.db_dir;
    if !db_path.exists() {
        std::fs::create_dir_all(db_path)?;
    }
    let db_file = db_path.join("endpoints.db");
    Ok(Arc::new(SqliteEndpointStorage::new(&db_file).await?))
}

/// A thread-safe in-memory storage for testing.
pub struct MockStorage {
    data: Arc<dashmap::DashMap<(String, String), SubstrateEndpoint>>,
}

impl Default for MockStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MockStorage {
    pub fn new() -> Self {
        Self { data: Arc::new(dashmap::DashMap::new()) }
    }
}

#[async_trait]
impl EndpointStorage for MockStorage {
    async fn load_all(&self) -> Result<Vec<(String, String, SubstrateEndpoint)>> {
        Ok(self
            .data
            .iter()
            .map(|e| (e.key().0.clone(), e.key().1.clone(), e.value().clone()))
            .collect())
    }
    async fn save(&self, sid: &str, iname: &str, ep: &SubstrateEndpoint) -> Result<()> {
        self.data.insert((sid.to_string(), iname.to_string()), ep.clone());
        Ok(())
    }
    async fn remove(&self, sid: &str, iname: &str) -> Result<()> {
        self.data.remove(&(sid.to_string(), iname.to_string()));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SubstrateEndpoint ↔ storage string mapping (single source of truth)
// ---------------------------------------------------------------------------

impl SubstrateEndpoint {
    /// Returns the discriminant key used in stable storage.
    pub fn storage_key(&self) -> &'static str {
        match self {
            Self::WasmChannel { .. } => "wasm",
            Self::PodmanSocket { .. } => "podman",
            Self::NativeHostChannel { .. } => "native",
        }
    }

    /// Returns the data payload stored alongside the key.
    fn storage_data(&self) -> &str {
        match self {
            Self::WasmChannel { service_id } => service_id,
            Self::PodmanSocket { socket_path } => socket_path,
            Self::NativeHostChannel { service_id } => service_id,
        }
    }
}

impl TryFrom<(&str, String)> for SubstrateEndpoint {
    type Error = anyhow::Error;

    fn try_from((key, data): (&str, String)) -> Result<Self> {
        match key {
            "wasm" => Ok(Self::WasmChannel { service_id: data }),
            "podman" => Ok(Self::PodmanSocket { socket_path: data }),
            "native" => Ok(Self::NativeHostChannel { service_id: data }),
            other => Err(anyhow::anyhow!("Unknown endpoint type in storage: {}", other)),
        }
    }
}

// ---------------------------------------------------------------------------

pub struct SqliteEndpointStorage {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl SqliteEndpointStorage {
    /// Create a new SqliteEndpointStorage with the given DB path.
    pub async fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let path = db_path.as_ref().to_owned();
        let conn = tokio::task::spawn_blocking(move || -> Result<rusqlite::Connection> {
            let conn = rusqlite::Connection::open(path)?;

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
            Ok(conn)
        })
        .await??;

        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }
}

fn lock_db(
    conn: &Arc<Mutex<rusqlite::Connection>>,
) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>> {
    conn.lock().map_err(|e| anyhow::anyhow!("Database connection mutex poisoned: {}", e))
}

#[async_trait]
impl EndpointStorage for SqliteEndpointStorage {
    async fn load_all(&self) -> Result<Vec<(String, String, SubstrateEndpoint)>> {
        let conn_arc = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<(String, String, SubstrateEndpoint)>> {
            let conn = lock_db(&conn_arc)?;
            let mut stmt = conn.prepare(
                "SELECT service_id, interface_name, endpoint_type, endpoint_data FROM local_endpoints",
            )?;

            let endpoints = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .filter_map(|(sid, iname, key, data)| {
                    SubstrateEndpoint::try_from((key.as_str(), data))
                        .ok()
                        .map(|ep| (sid, iname, ep))
                })
                .collect();

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
        let e_data = endpoint.storage_data().to_string();

        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock_db(&conn_arc)?;
            conn.execute(
                "INSERT INTO local_endpoints (service_id, interface_name, endpoint_type, endpoint_data)
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

        tokio::task::spawn_blocking(move || -> Result<()> {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    async fn make_store() -> (SqliteEndpointStorage, tempfile::TempDir) {
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

    #[test]
    fn test_storage_key_roundtrip() {
        let cases = [
            SubstrateEndpoint::WasmChannel { service_id: "a".to_string() },
            SubstrateEndpoint::PodmanSocket { socket_path: "/tmp/x".to_string() },
            SubstrateEndpoint::NativeHostChannel { service_id: "b".to_string() },
        ];
        for ep in &cases {
            let key = ep.storage_key();
            let data = ep.storage_data().to_string();
            let back = SubstrateEndpoint::try_from((key, data)).unwrap();
            assert_eq!(ep.storage_key(), back.storage_key());
        }
    }
}
