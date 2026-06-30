//! Stable storage abstraction and persistence backend
//!
//! Defines the `EndpointStorage` trait and implements `SQLite` persistence
//! and thread-safe in-memory mock storage for the local `EndpointRegistry`.

use std::{fmt::Debug, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;

use crate::local_registry::SubstrateEndpoint;

/// A trait abstracting stable storage for the `EndpointRegistry`.
#[async_trait]
pub trait EndpointStorage: Send + Sync {
    /// Load all endpoints from stable storage. Returns a vector of
    /// (`service_id`, `interface_name`, endpoint).
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

/// A thread-safe in-memory storage for testing.
#[derive(Debug)]
pub struct MockStorage {
    data: Arc<DashMap<(String, String), SubstrateEndpoint>>,
}

impl Default for MockStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MockStorage {
    #[must_use]
    pub fn new() -> Self {
        Self { data: Arc::new(DashMap::new()) }
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
    #[must_use]
    pub const fn storage_key(&self) -> &'static str {
        match self {
            Self::WasmChannel { .. } => "wasm",
            Self::PodmanSocket { .. } => "podman",
            Self::NativeHostChannel { .. } => "native",
            Self::TcpHostPort { .. } => "tcp",
        }
    }

    /// Returns the data payload stored alongside the key.
    pub fn storage_data(&self) -> String {
        match self {
            Self::WasmChannel { service_id } => service_id.clone(),
            Self::PodmanSocket { socket_path } => socket_path.clone(),
            Self::NativeHostChannel { service_id } => service_id.clone(),
            Self::TcpHostPort { host, port } => format!("{host}:{port}"),
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
            "tcp" => {
                let (host, port_str) = data
                    .split_once(':')
                    .ok_or_else(|| anyhow::anyhow!("Invalid TCP endpoint data: {data}"))?;
                let port = port_str.parse().map_err(|e| {
                    anyhow::anyhow!("Invalid port in TCP endpoint data: {data} ({e})")
                })?;
                Ok(Self::TcpHostPort { host: host.to_string(), port })
            }
            other => Err(anyhow::anyhow!("Unknown endpoint type in storage: {other}")),
        }
    }
}
