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

impl SignedEndpointInfo {
    /// Verifies the signature on this endpoint info using the public key embedded in its service_id.
    pub fn verify(&self) -> Result<(), anyhow::Error> {
        let pubkey = syneroym_identity::substrate::resolve_did_key(&self.info.service_id)
            .map_err(|e| anyhow::anyhow!("Failed to parse public key from service_id DID: {e}"))?;

        let sig_bytes = z32::decode(self.signature.as_bytes())
            .map_err(|_| anyhow::anyhow!("Invalid z-base-32 signature encoding"))?;

        let signature = ed25519_dalek::Signature::from_slice(&sig_bytes)
            .map_err(|e| anyhow::anyhow!("Invalid signature length: {e}"))?;

        let info_value = serde_json::to_value(&self.info)?;
        let canonical_value = syneroym_identity::substrate::canonicalize_json_value(&info_value);
        let canonical_string = serde_json::to_string(&canonical_value)?;

        use ed25519_dalek::Verifier;
        pubkey.verify(canonical_string.as_bytes(), &signature).map_err(Into::into)
    }
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
        // Always ask the registry NOT to resolve mechanisms, because if the registry
        // mutates the payload to inject mechanisms, it will invalidate the signature.
        // We will perform the resolution locally instead.
        let url = format!("{registry_url}/lookup/{id}?resolve=false");

        tracing::debug!("Registry lookup: {}", url);

        let response = client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Registry lookup failed with status: {} for ID: {}",
                response.status(),
                id
            ));
        }

        let mut info = response.json::<SignedEndpointInfo>().await?;

        info.verify().map_err(|e| {
            anyhow::anyhow!(
                "Registry returned an invalid or spoofed endpoint info for {}: {}",
                id,
                e
            )
        })?;

        // Perform local resolution
        if resolve && info.info.endpoint_type == EndpointType::Service {
            tracing::debug!("Resolving substrate mechanisms for service {}", info.info.service_id);
            let sub_url = format!("{registry_url}/lookup/{}?resolve=false", info.info.substrate_id);
            let sub_response = client.get(&sub_url).send().await?;

            if !sub_response.status().is_success() {
                return Err(anyhow::anyhow!(
                    "Failed to resolve substrate {} for service {}",
                    info.info.substrate_id,
                    info.info.service_id
                ));
            }

            let sub_info = sub_response.json::<SignedEndpointInfo>().await?;
            sub_info
                .verify()
                .map_err(|e| anyhow::anyhow!("Registry returned invalid substrate info: {}", e))?;

            info.info.mechanisms = sub_info.info.mechanisms;
        }

        Ok(info)
    }
}
