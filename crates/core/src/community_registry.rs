use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointType {
    Substrate,
    Service,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointInfo {
    pub service_id: String,   // e.g. substrate did:key
    pub substrate_id: String, // For substrate itself, it's the same as service_id
    pub endpoint_type: EndpointType,
    pub relay_url: Option<String>,
    pub endpoint_addr_bytes: Vec<u8>, // serialized iroh::EndpointAddr
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedEndpointInfo {
    pub info: EndpointInfo,
    pub signature: String, // z32 encoded ed25519 signature
}
