use anyhow::Result;
use dashmap::DashMap;
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use std::sync::Arc;

/// A deployable entity within the Substrate.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SubstrateEndpoint {
    /// A WASM component communicating via wRPC channels
    WasmChannel { channel_id: String },
    /// A containerized service running via Podman
    PodmanSocket { socket_path: String },
    /// A native Rust host capability or service (e.g. SubstrateService)
    NativeHostChannel { channel_id: String },
}

/// The Endpoint Registry tracks where local Services are currently executing.
/// It acts as Internal Micro-Discovery.
#[derive(Debug, Clone)]
pub struct EndpointRegistry {
    /// Thread-safe shared map of service-id to LocalEndpoint
    active_endpoints: Arc<DashMap<String, SubstrateEndpoint>>,
    /// Stable storage connection for persistence
    db_pool: SqlitePool,
}

impl EndpointRegistry {
    /// Create a new Endpoint Registry with SQLite stable storage.
    pub async fn new(db_url: &str) -> Result<Self> {
        let db_pool = SqlitePoolOptions::new().max_connections(5).connect(db_url).await?;

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

        let registry = Self { active_endpoints: Arc::new(DashMap::new()), db_pool };

        registry.load_from_db().await?;

        Ok(registry)
    }

    /// Load endpoints from stable storage into memory map on startup
    async fn load_from_db(&self) -> Result<()> {
        use sqlx::Row;
        let rows =
            sqlx::query("SELECT service_id, endpoint_type, endpoint_data FROM local_endpoints")
                .fetch_all(&self.db_pool)
                .await?;

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
            self.active_endpoints.insert(service_id, endpoint);
        }
        Ok(())
    }

    /// Register a local service. Stores it in memory and stable storage.
    pub async fn register(&self, service_id: String, endpoint: SubstrateEndpoint) -> Result<()> {
        let (e_type, e_data) = match &endpoint {
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
        .bind(service_id.clone())
        .bind(e_type)
        .bind(e_data)
        .execute(&self.db_pool)
        .await?;

        self.active_endpoints.insert(service_id, endpoint);
        Ok(())
    }

    /// Lookup a destination for an incoming request
    pub fn lookup(&self, service_id: &str) -> Option<SubstrateEndpoint> {
        self.active_endpoints.get(service_id).map(|e| e.clone())
    }

    /// Remove a service from registry
    pub async fn remove(&self, service_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM local_endpoints WHERE service_id = ?")
            .bind(service_id)
            .execute(&self.db_pool)
            .await?;
        self.active_endpoints.remove(service_id);
        Ok(())
    }
}
