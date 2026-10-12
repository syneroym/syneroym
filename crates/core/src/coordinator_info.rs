//! The answer of `GET /v1/info` on an Iroh coordinator. A substrate reads it
//! before it uses a relay from a curated list, and operators read it to see
//! the coordinator's health.

use serde::{Deserialize, Serialize};

/// The path of the info endpoint on the coordinator's info server.
pub const INFO_PATH: &str = "/v1/info";

/// Whether the coordinator's relay takes more clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinatorStatus {
    Healthy,
    /// The relay has at least `max_relay_clients` connections. A substrate
    /// picks another relay.
    AtCapacity,
    /// A value this build does not know, or no value.
    #[default]
    #[serde(other)]
    Unknown,
}

/// The identity of the host substrate. Not reported yet: the field is `null`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IdentityInfo {
    pub did: String,
    pub controller_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConnectionsInfo {
    /// Open Syneroym connections to the coordinator's endpoint. Not relay
    /// connections.
    pub active: usize,
    /// `max_connections`.
    pub cap: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsInfo {
    pub cert_expiry_days: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RelayInfo {
    /// The coordinator runs a relay server.
    pub online: bool,
    /// Open relay connections now. `None` when there is no relay server.
    #[serde(default)]
    pub clients: Option<u64>,
    /// `max_relay_clients`.
    #[serde(default)]
    pub max_clients: Option<u64>,
}

/// The coordinator's own record in its community registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryInfo {
    /// The DID the record is registered under. New at every start.
    pub did: String,
    /// The registry accepted the last registration. Registration repeats
    /// every hour after a success and every minute after a failure.
    pub registered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorInfo {
    pub endpoint_addr_bytes: Vec<u8>, // serde_json-encoded EndpointAddr
    /// The coordinator's Iroh endpoint id. Clients dial it with this id.
    pub substrate_id: String,
    pub relay_url: Option<String>, // Local relay URL hosted by this coordinator
    pub parent_coordinator_url: Option<String>, // Parent coordinator URL

    #[serde(default)]
    pub status: CoordinatorStatus,
    #[serde(default)]
    pub identity: Option<IdentityInfo>,
    #[serde(default)]
    pub connections: Option<ConnectionsInfo>,
    #[serde(default)]
    pub tls: Option<TlsInfo>,
    #[serde(default)]
    pub relay: Option<RelayInfo>,
    /// `None` when the coordinator does not register itself.
    #[serde(default)]
    pub registry: Option<RegistryInfo>,
}

#[cfg(test)]
mod tests;
