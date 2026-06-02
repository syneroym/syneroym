//! Community Registry Client and Types
//!
//! Provides structures and client methods for registering, querying, and
//! resolving service/substrate endpoints in the Syneroym community registry.

use serde::{Deserialize, Serialize};

/// Default time-to-live for registry entries, aligned with BEP 0044 DHT expiry defaults.
pub const DEFAULT_REGISTRY_TTL_SECS: u64 = 7200; // 2 hours

/// Interval at which substrates republish their endpoints to prevent them from expiring.
pub const HEARTBEAT_INTERVAL_SECS: u64 = 3600; // 1 hour

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointType {
    Substrate,
    Service,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointMechanism {
    Iroh { endpoint_addr_bytes: Vec<u8>, relay_url: Option<String> },
    WebRtc { peer_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointInfo {
    pub service_id: String,   // e.g. substrate did:key
    pub substrate_id: String, // For substrate itself, it's the same as service_id
    pub endpoint_type: EndpointType,
    pub mechanisms: Vec<EndpointMechanism>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(default)]
    pub is_private: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedEndpointInfo {
    pub info: EndpointInfo,
    pub signature: String, // z32 encoded ed25519 signature
}

#[derive(Debug)]
pub struct RegistryClient;

impl RegistryClient {
    /// Look up a service or substrate in the community registry.
    /// Handles both full DIDs and shorthash aliases.
    /// If `resolve` is true, it will follow service-to-substrate mappings to get mechanisms.
    pub async fn lookup(
        registry_url: &str,
        id: &str,
        resolve: bool,
    ) -> Result<SignedEndpointInfo, anyhow::Error> {
        let client = reqwest::Client::new();
        let url = format!("{registry_url}/lookup/{id}?resolve={resolve}");

        tracing::debug!("Registry lookup: {}", url);

        let response = client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Registry lookup failed with status: {} for ID: {}",
                response.status(),
                id
            ));
        }

        let info = response.json::<SignedEndpointInfo>().await?;
        Ok(info)
    }
}
