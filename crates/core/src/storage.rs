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
    let db_path = config.app_local_data_dir.join(&config.storage.db_dir);
    if !db_path.exists() {
        std::fs::create_dir_all(&db_path)?;
    }
    let db_file = db_path.join("endpoints.db");
    Ok(Arc::new(SqliteEndpointStorage::new(&db_file).await?))
}

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

#[async_trait]
impl EndpointStorage for SqliteEndpointStorage {
    async fn load_all(&self) -> Result<Vec<(String, String, SubstrateEndpoint)>> {
        let conn_arc = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<(String, String, SubstrateEndpoint)>> {
            let conn = conn_arc.lock().unwrap();
            let mut stmt = conn.prepare("SELECT service_id, interface_name, endpoint_type, endpoint_data FROM local_endpoints")?;

            let endpoint_iter = stmt.query_map([], |row| {
                let service_id: String = row.get(0)?;
                let interface_name: String = row.get(1)?;
                let endpoint_type: String = row.get(2)?;
                let endpoint_data: String = row.get(3)?;
                Ok((service_id, interface_name, endpoint_type, endpoint_data))
            })?;

            let mut endpoints = Vec::new();
            for item in endpoint_iter {
                let (service_id, interface_name, endpoint_type, endpoint_data) = item?;

                let endpoint = match endpoint_type.as_str() {
                    "wasm" => SubstrateEndpoint::WasmChannel { channel_details: endpoint_data },
                    "podman" => SubstrateEndpoint::PodmanSocket { socket_path: endpoint_data },
                    "native" => SubstrateEndpoint::NativeHostChannel { channel_details: endpoint_data },
                    _ => continue,
                };
                endpoints.push((service_id, interface_name, endpoint));
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
        let (e_type, e_data) = match endpoint {
            SubstrateEndpoint::WasmChannel { channel_details } => ("wasm", channel_details.clone()),
            SubstrateEndpoint::PodmanSocket { socket_path } => ("podman", socket_path.clone()),
            SubstrateEndpoint::NativeHostChannel { channel_details } => {
                ("native", channel_details.clone())
            }
        };

        let conn_arc = self.conn.clone();
        let sid = service_id.to_string();
        let iname = interface_name.to_string();

        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn_arc.lock().unwrap();
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
            let conn = conn_arc.lock().unwrap();
            conn.execute(
                "DELETE FROM local_endpoints WHERE service_id = ?1 AND interface_name = ?2",
                params![sid, iname],
            )?;
            Ok(())
        })
        .await?
    }
}
