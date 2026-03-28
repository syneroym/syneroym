use std::sync::Arc;

use crate::{config::SubstrateConfig, registry::SubstrateEndpoint};
use anyhow::Result;

use async_trait::async_trait;

/// A trait abstracting stable storage for the EndpointRegistry.
#[async_trait]
pub trait EndpointStorage: Send + Sync {
    /// Load all endpoints from stable storage.
    async fn load_all(&self) -> Result<Vec<(String, SubstrateEndpoint)>>;

    /// Save an endpoint into stable storage.
    async fn save(&self, service_id: &str, endpoint: &SubstrateEndpoint) -> Result<()>;

    /// Remove an endpoint from stable storage.
    async fn remove(&self, service_id: &str) -> Result<()>;
}

pub async fn init_store(config: &SubstrateConfig) -> Result<Arc<dyn EndpointStorage>> {
    let db_path = config.app_local_data_dir.join(&config.storage.db_dir);
    if !db_path.exists() {
        std::fs::create_dir_all(&db_path)?;
    }
    let db_url = format!("sqlite://{}/endpoints.db?mode=rwc", db_path.to_string_lossy());
    Ok(Arc::new(SqliteEndpointStorage::new(&db_url).await?))
}

pub struct SqliteEndpointStorage {
    db_pool: sqlx::SqlitePool,
}

impl SqliteEndpointStorage {
    /// Create a new SqliteEndpointStorage with the given DB URL.
    pub async fn new(db_url: &str) -> Result<Self> {
        let db_pool =
            sqlx::sqlite::SqlitePoolOptions::new().max_connections(5).connect(db_url).await?;

        // Basic schema creation for endpoints
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS local_endpoints (
                service_id TEXT PRIMARY KEY,
                endpoint_type TEXT NOT NULL,
                endpoint_data TEXT NOT NULL
            );",
        )
        .execute(&db_pool)
        .await?;

        Ok(Self { db_pool })
    }
}

#[async_trait]
impl EndpointStorage for SqliteEndpointStorage {
    async fn load_all(&self) -> Result<Vec<(String, SubstrateEndpoint)>> {
        use sqlx::Row;
        let rows =
            sqlx::query("SELECT service_id, endpoint_type, endpoint_data FROM local_endpoints")
                .fetch_all(&self.db_pool)
                .await?;

        let mut endpoints = Vec::new();
        for row in rows {
            let service_id: String = row.get("service_id");
            let endpoint_type: String = row.get("endpoint_type");
            let endpoint_data: String = row.get("endpoint_data");

            let endpoint = match endpoint_type.as_str() {
                "wasm" => SubstrateEndpoint::WasmChannel { channel_id: endpoint_data },
                "podman" => SubstrateEndpoint::PodmanSocket { socket_path: endpoint_data },
                "native" => SubstrateEndpoint::NativeHostChannel { channel_id: endpoint_data },
                _ => continue,
            };
            endpoints.push((service_id, endpoint));
        }
        Ok(endpoints)
    }

    async fn save(&self, service_id: &str, endpoint: &SubstrateEndpoint) -> Result<()> {
        let (e_type, e_data) = match endpoint {
            SubstrateEndpoint::WasmChannel { channel_id } => ("wasm", channel_id.clone()),
            SubstrateEndpoint::PodmanSocket { socket_path } => ("podman", socket_path.clone()),
            SubstrateEndpoint::NativeHostChannel { channel_id } => ("native", channel_id.clone()),
        };

        sqlx::query(
            "INSERT INTO local_endpoints (service_id, endpoint_type, endpoint_data)
             VALUES (?, ?, ?)
             ON CONFLICT(service_id) DO UPDATE SET
                endpoint_type = excluded.endpoint_type,
                endpoint_data = excluded.endpoint_data",
        )
        .bind(service_id)
        .bind(e_type)
        .bind(e_data)
        .execute(&self.db_pool)
        .await?;

        Ok(())
    }

    async fn remove(&self, service_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM local_endpoints WHERE service_id = ?")
            .bind(service_id)
            .execute(&self.db_pool)
            .await?;
        Ok(())
    }
}
