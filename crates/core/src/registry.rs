use crate::storage::EndpointStorage;
use anyhow::Result;
use dashmap::DashMap;
use std::sync::Arc;

/// A deployable entity within the Substrate.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SubstrateEndpoint {
    /// A WASM component communicating via wRPC channels
    WasmChannel { channel_details: String },
    /// A containerized service running via Podman
    PodmanSocket { socket_path: String },
    /// A native Rust host capability or service (e.g. SubstrateService)
    NativeHostChannel { channel_details: String },
}

/// The Endpoint Registry tracks where local Services are currently executing.
/// It acts as Internal Micro-Discovery.
#[derive(Clone)]
pub struct EndpointRegistry {
    /// Thread-safe shared map of (service_id, interface_name) to LocalEndpoint
    active_endpoints: Arc<DashMap<(String, String), SubstrateEndpoint>>,
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

        for (service_id, interface_name, endpoint) in endpoints {
            self.active_endpoints.insert((service_id, interface_name), endpoint);
        }
        Ok(())
    }

    /// Register a local service. Stores it in memory and stable storage.
    pub async fn register(
        &self,
        service_id: String,
        interface_name: String,
        endpoint: SubstrateEndpoint,
    ) -> Result<()> {
        self.storage.save(&service_id, &interface_name, &endpoint).await?;
        self.active_endpoints.insert((service_id, interface_name), endpoint);
        Ok(())
    }

    /// Lookup a destination for an incoming request
    pub fn lookup(&self, service_id: &str, interface_name: &str) -> Option<SubstrateEndpoint> {
        self.active_endpoints
            .get(&(service_id.to_string(), interface_name.to_string()))
            .map(|e| e.clone())
    }

    /// Remove a service from registry
    pub async fn remove(&self, service_id: &str, interface_name: &str) -> Result<()> {
        self.storage.remove(service_id, interface_name).await?;
        self.active_endpoints.remove(&(service_id.to_string(), interface_name.to_string()));
        Ok(())
    }

    /// Returns a list of all currently registered endpoints
    pub fn get_all_endpoints(&self) -> Vec<(String, String, SubstrateEndpoint)> {
        self.active_endpoints
            .iter()
            .map(|entry| {
                let (service_id, interface_name) = entry.key().clone();
                (service_id, interface_name, entry.value().clone())
            })
            .collect()
    }
}
