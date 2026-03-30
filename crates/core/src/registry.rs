use crate::storage::EndpointStorage;
use anyhow::Result;
use dashmap::DashMap;
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
#[derive(Clone)]
pub struct EndpointRegistry {
    /// Thread-safe shared map of service-id to LocalEndpoint
    active_endpoints: Arc<DashMap<String, SubstrateEndpoint>>,
    /// Stable storage connection for persistence
    storage: Arc<dyn EndpointStorage>,
}

impl std::fmt::Debug for EndpointRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EndpointRegistry")
            .field("active_endpoints", &self.active_endpoints)
            // storage does not implement Debug, so we skip it
            .finish()
    }
}

impl EndpointRegistry {
    /// Create a new Endpoint Registry with the given stable storage.
    pub async fn new(storage: Arc<dyn EndpointStorage>) -> Result<Self> {
        let registry = Self { active_endpoints: Arc::new(DashMap::new()), storage };

        registry.load_from_db().await?;

        Ok(registry)
    }

    /// Load endpoints from stable storage into memory map on startup
    async fn load_from_db(&self) -> Result<()> {
        let endpoints = self.storage.load_all().await?;

        for (service_id, endpoint) in endpoints {
            self.active_endpoints.insert(service_id, endpoint);
        }
        Ok(())
    }

    /// Register a local service. Stores it in memory and stable storage.
    pub async fn register(&self, service_id: String, endpoint: SubstrateEndpoint) -> Result<()> {
        self.storage.save(&service_id, &endpoint).await?;
        self.active_endpoints.insert(service_id, endpoint);
        Ok(())
    }

    /// Lookup a destination for an incoming request
    pub fn lookup(&self, service_id: &str) -> Option<SubstrateEndpoint> {
        self.active_endpoints.get(service_id).map(|e| e.clone())
    }

    /// Remove a service from registry
    pub async fn remove(&self, service_id: &str) -> Result<()> {
        self.storage.remove(service_id).await?;
        self.active_endpoints.remove(service_id);
        Ok(())
    }

    /// Returns a list of all currently registered endpoints
    pub fn get_all_endpoints(&self) -> Vec<(String, SubstrateEndpoint)> {
        self.active_endpoints
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }
}
