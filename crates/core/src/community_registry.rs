//! Community Registry Client and Types
//!
//! Provides structures and client methods for registering, querying, and
//! resolving service/substrate endpoints in the Syneroym community registry.

use serde::{Deserialize, Serialize};

/// Default time-to-live for registry entries, aligned with BEP 0044 DHT expiry defaults.
pub const DEFAULT_REGISTRY_TTL_SECS: u64 = 7200; // 2 hours

/// Interval at which substrates republish their endpoints to prevent them from expiring.
pub const HEARTBEAT_INTERVAL_SECS: u64 = 3600; // 1 hour

/// Internal pkarr DHT DNS name used in published packets
pub const PKARR_DNS_NAME: &str = "syneroym";

/// Internal pkarr DHT packet TTL. Matches heartbeat interval so records expire if not refreshed.
pub const PKARR_TTL: u32 = HEARTBEAT_INTERVAL_SECS as u32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointType {
    Substrate,
    Service,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointMechanism {
    Iroh {
        #[serde(with = "hex")]
        endpoint_addr_bytes: Vec<u8>,
        relay_url: Option<String>,
    },
    WebRtc {
        peer_id: String,
    },
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
    pub pkarr_packet_hex: String, // Hex encoded pkarr::SignedPacket bytes
}

impl EndpointInfo {
    pub fn sign(
        self,
        identity: &syneroym_identity::Identity,
    ) -> Result<SignedEndpointInfo, anyhow::Error> {
        let keypair = pkarr::Keypair::from_secret_key(&identity.to_bytes());
        let json_str = serde_json::to_string(&self)?;
        let txt_rdata = pkarr::dns::rdata::TXT::try_from(json_str.as_str())
            .map_err(|e| anyhow::anyhow!("Failed to construct TXT record: {e}"))?;
        let name = pkarr::dns::Name::new(PKARR_DNS_NAME)
            .map_err(|e| anyhow::anyhow!("Failed to create DNS name: {e}"))?;

        let records = vec![pkarr::dns::ResourceRecord::new(
            name,
            pkarr::dns::CLASS::IN,
            PKARR_TTL,
            pkarr::dns::rdata::RData::TXT(txt_rdata),
        )];
        let timestamp = pkarr::Timestamp::now();
        let signed_packet = pkarr::SignedPacket::new(&keypair, &records, timestamp)
            .map_err(|e| anyhow::anyhow!("Failed to sign pkarr packet: {e}"))?;
        let pkarr_packet_hex = hex::encode(signed_packet.to_relay_payload());
        Ok(SignedEndpointInfo { info: self, pkarr_packet_hex })
    }
}

impl SignedEndpointInfo {
    /// Verifies the pkarr packet using the public key embedded in its service_id.
    pub fn verify(&self) -> Result<(), anyhow::Error> {
        let pubkey = syneroym_identity::substrate::resolve_did_key(&self.info.service_id)
            .map_err(|e| anyhow::anyhow!("Failed to parse public key from service_id DID: {e}"))?;

        let expected_pkarr_pubkey = pkarr::PublicKey::try_from(pubkey.as_bytes())
            .map_err(|e| anyhow::anyhow!("Invalid ed25519 pubkey for pkarr: {e}"))?;

        let packet_bytes = hex::decode(&self.pkarr_packet_hex)
            .map_err(|_| anyhow::anyhow!("Invalid hex encoding for pkarr packet"))?;

        let bytes_obj = bytes::Bytes::from(packet_bytes);
        let signed_packet =
            pkarr::SignedPacket::from_relay_payload(&expected_pkarr_pubkey, &bytes_obj)
                .map_err(|e| anyhow::anyhow!("Invalid pkarr packet signature or structure: {e}"))?;

        if signed_packet.public_key() != expected_pkarr_pubkey {
            return Err(anyhow::anyhow!("Signed packet public key does not match service_id"));
        }

        // Extract the TXT record to ensure it matches the service_id
        let mut found_txt = false;
        for answer in signed_packet.resource_records(PKARR_DNS_NAME) {
            if let pkarr::dns::rdata::RData::TXT(txt) = &answer.rdata
                && let Ok(full_string) = String::try_from(txt.clone())
                && let Ok(parsed_info) = serde_json::from_str::<EndpointInfo>(&full_string)
                && parsed_info.service_id == self.info.service_id
            {
                found_txt = true;
                break;
            }
        }

        if !found_txt {
            return Err(anyhow::anyhow!(
                "pkarr packet does not contain the corresponding EndpointInfo"
            ));
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct RegistryClient {
    dht_client: Option<pkarr::Client>,
    registry_url: Option<String>,
}

impl RegistryClient {
    pub fn new(enable_dht: bool, registry_url: Option<String>) -> Self {
        let dht_client = if enable_dht { pkarr::Client::builder().build().ok() } else { None };
        Self { dht_client, registry_url }
    }

    /// Registers the endpoint to the DHT and optionally the HTTP registry.
    pub async fn register(&self, signed_info: &SignedEndpointInfo) -> anyhow::Result<()> {
        let mut http_success = self.registry_url.is_none();

        if let Some(url) = &self.registry_url {
            let client = reqwest::Client::new();
            let register_url = format!("{url}/register");
            tracing::debug!("Registry register: {}", register_url);

            match client.post(&register_url).json(signed_info).send().await {
                Ok(response) if response.status().is_success() => {
                    http_success = true;
                }
                Ok(response) => {
                    tracing::warn!("HTTP registry returned error status: {}", response.status());
                }
                Err(e) => {
                    tracing::warn!("Failed to connect to HTTP registry: {}", e);
                }
            }
        }

        if !http_success {
            return Err(anyhow::anyhow!("Failed to register endpoint via HTTP registry"));
        }

        // Publish to DHT (fire-and-forget in background) if HTTP succeeded or wasn't configured
        if let Some(dht) = &self.dht_client {
            tracing::debug!("Publishing to Mainline DHT (background)");
            let packet_bytes = hex::decode(&signed_info.pkarr_packet_hex)?;
            let bytes_obj = bytes::Bytes::from(packet_bytes);
            let pubkey =
                syneroym_identity::substrate::resolve_did_key(&signed_info.info.service_id)?;
            let pkarr_pubkey = pkarr::PublicKey::try_from(pubkey.as_bytes())?;
            let signed_packet = pkarr::SignedPacket::from_relay_payload(&pkarr_pubkey, &bytes_obj)?;

            let dht = dht.clone();
            tokio::spawn(async move {
                if let Err(e) = dht.publish(&signed_packet, None).await {
                    tracing::warn!("Failed to publish to DHT: {}", e);
                } else {
                    tracing::debug!("Successfully published to Mainline DHT");
                }
            });
        }

        Ok(())
    }

    /// Look up a service or substrate in the community registry.
    /// Handles both full DIDs and shorthash aliases.
    /// If `resolve` is true, it will follow service-to-substrate mappings to get mechanisms.
    pub async fn lookup(
        &self,
        id: &str,
        resolve: bool,
    ) -> Result<SignedEndpointInfo, anyhow::Error> {
        let mut result = None;

        // Try HTTP registry first
        if let Some(url) = &self.registry_url {
            let client = reqwest::Client::new();
            let lookup_url = format!("{url}/lookup/{id}?resolve=false");
            tracing::debug!("Registry lookup: {}", lookup_url);

            if let Ok(response) = client.get(&lookup_url).send().await
                && response.status().is_success()
                && let Ok(info) = response.json::<SignedEndpointInfo>().await
                && info.verify().is_ok()
            {
                result = Some(info);
            }
        }

        // Try DHT if HTTP failed or wasn't configured
        let is_dht_lookup = result.is_none();
        if result.is_none()
            && let Some(dht) = &self.dht_client
        {
            // Note: DHT lookups require a public key, so shorthash aliases won't work purely on DHT
            if let Ok(pubkey) = syneroym_identity::substrate::resolve_did_key(id) {
                if let Ok(pkarr_pubkey) = pkarr::PublicKey::try_from(pubkey.as_bytes()) {
                    tracing::debug!("Falling back to DHT lookup for {}", id);
                    if let Some(signed_packet) = dht.resolve(&pkarr_pubkey).await {
                        // Extract the EndpointInfo
                        let mut found_info = None;
                        for answer in signed_packet.resource_records(PKARR_DNS_NAME) {
                            if let pkarr::dns::rdata::RData::TXT(txt) = &answer.rdata
                                && let Ok(full_string) = String::try_from(txt.clone())
                                && let Ok(parsed_info) =
                                    serde_json::from_str::<EndpointInfo>(&full_string)
                                && parsed_info.service_id == id
                            {
                                found_info = Some(parsed_info);
                                break;
                            }
                        }

                        if let Some(info) = found_info {
                            let pkarr_packet_hex = hex::encode(signed_packet.to_relay_payload());
                            result = Some(SignedEndpointInfo { info, pkarr_packet_hex });
                        }
                    }
                }
            } else {
                tracing::warn!("Cannot perform DHT lookup for non-DID identifier: {}", id);
            }
        }

        let mut info = result.ok_or_else(|| {
            anyhow::anyhow!("Endpoint not found in registry or DHT for ID: {}", id)
        })?;

        // Proactively backfill cache if it was found via DHT
        if is_dht_lookup && self.registry_url.is_some() {
            // we ignore failures on cache backfilling
            let _ = self.register(&info).await;
        }

        // Perform local resolution if requested
        if resolve && info.info.endpoint_type == EndpointType::Service {
            tracing::debug!("Resolving substrate mechanisms for service {}", info.info.service_id);
            let sub_info = Box::pin(self.lookup(&info.info.substrate_id, false)).await?;
            info.info.mechanisms = sub_info.info.mechanisms;
        }

        Ok(info)
    }
}
