use crate::storage::EndpointStorage;
use anyhow::Result;
use dashmap::DashMap;
use std::sync::Arc;

/// A deployable entity within the Substrate.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SubstrateEndpoint {
    /// A WASM component communicating via wRPC channels
    WasmChannel { service_id: String },
    /// A containerized service running via Podman
    PodmanSocket { socket_path: String },
    /// A native Rust host capability or service (e.g. SubstrateService)
    NativeHostChannel { service_id: String },
}

/// The Endpoint Registry tracks where local Services are currently executing.
/// It acts as Internal Micro-Discovery.
#[derive(Clone)]
pub struct EndpointRegistry {
    /// Thread-safe shared map of (service_id, interface_name) to LocalEndpoint
    active_endpoints: Arc<DashMap<(String, String), SubstrateEndpoint>>,
    /// Secondary map for fast lookup by interface hash: (service_id, interface_hash) -> interface_name
    interface_hashes: Arc<DashMap<(String, String), String>>,
    /// Stable storage connection for persistence
    storage: Arc<dyn EndpointStorage>,
}

impl std::fmt::Debug for EndpointRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EndpointRegistry")
            .field("active_endpoints", &self.active_endpoints)
            .field("interface_hashes", &self.interface_hashes)
            // storage does not implement Debug, so we skip it
            .finish()
    }
}

impl EndpointRegistry {
    /// Create a new Endpoint Registry with the given stable storage.
    pub async fn new(storage: Arc<dyn EndpointStorage>) -> Result<Self> {
        let registry = Self {
            active_endpoints: Arc::new(DashMap::new()),
            interface_hashes: Arc::new(DashMap::new()),
            storage,
        };

        registry.load_from_db().await?;

        Ok(registry)
    }

    /// Load endpoints from stable storage into memory map on startup
    async fn load_from_db(&self) -> Result<()> {
        let endpoints = self.storage.load_all().await?;

        for (service_id, interface_name, endpoint) in endpoints {
            let hash = crate::util::short_hash(&interface_name);
            self.interface_hashes.insert((service_id.clone(), hash), interface_name.clone());
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

        let hash = crate::util::short_hash(&interface_name);
        self.interface_hashes.insert((service_id.clone(), hash), interface_name.clone());
        self.active_endpoints.insert((service_id, interface_name), endpoint);
        Ok(())
    }

    /// Lookup a destination for an incoming request.
    /// Returns the endpoint and the canonical interface name it was registered under.
    /// The canonical interface name may differ from `interface_name` when a short hash is provided.
    pub fn lookup(
        &self,
        service_id: &str,
        interface_name: &str,
    ) -> Option<(SubstrateEndpoint, String)> {
        // First try exact match
        if let Some(ep) = self
            .active_endpoints
            .get(&(service_id.to_string(), interface_name.to_string()))
            .map(|e| e.clone())
        {
            return Some((ep, interface_name.to_string()));
        }

        // Then try hash match
        if let Some(canonical) =
            self.interface_hashes.get(&(service_id.to_string(), interface_name.to_string()))
            && let Some(ep) = self
                .active_endpoints
                .get(&(service_id.to_string(), canonical.clone()))
                .map(|e| e.clone())
        {
            return Some((ep, canonical.clone()));
        }

        None
    }

    /// Remove a service from registry
    pub async fn remove(&self, service_id: &str, interface_name: &str) -> Result<()> {
        self.storage.remove(service_id, interface_name).await?;

        let hash = crate::util::short_hash(interface_name);
        self.interface_hashes.remove(&(service_id.to_string(), hash));
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

    /// Creates a mock registry with in-memory storage for testing.
    pub fn new_mock(storage: Arc<crate::storage::MockStorage>) -> Self {
        Self {
            active_endpoints: Arc::new(DashMap::new()),
            interface_hashes: Arc::new(DashMap::new()),
            storage,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MockStorage;

    #[tokio::test]
    async fn test_registry_lifecycle() {
        let storage = Arc::new(MockStorage::new());
        let registry = EndpointRegistry::new(storage.clone()).await.unwrap();

        // 1. Register
        let service = "test-service".to_string();
        let iface = "health".to_string();
        let endpoint = SubstrateEndpoint::WasmChannel { service_id: service.clone() };

        registry.register(service.clone(), iface.clone(), endpoint.clone()).await.unwrap();

        // 2. Lookup
        let (found, canonical) = registry.lookup(&service, &iface).unwrap();
        assert_eq!(canonical, iface);
        match found {
            SubstrateEndpoint::WasmChannel { service_id } => assert_eq!(service_id, service),
            _ => panic!("Wrong endpoint type"),
        }

        // 3. Persistence check (new registry instance with same storage)
        let registry2 = EndpointRegistry::new(storage).await.unwrap();
        assert!(registry2.lookup(&service, &iface).is_some());

        // 4. Remove
        registry2.remove(&service, &iface).await.unwrap();
        assert!(registry2.lookup(&service, &iface).is_none());
    }
}
