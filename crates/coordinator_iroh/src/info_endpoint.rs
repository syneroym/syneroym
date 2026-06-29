use std::{fs, sync::Arc};

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use x509_parser::{
    pem::parse_x509_pem,
    prelude::{FromDer, X509Certificate},
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IdentityInfo {
    pub did: String,
    pub controller_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConnectionsInfo {
    pub active: usize,
    pub cap: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsInfo {
    pub cert_expiry_days: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RelayInfo {
    pub online: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorInfo {
    pub endpoint_addr_bytes: Vec<u8>, // serde_json-encoded EndpointAddr
    pub substrate_id: String,
    pub relay_url: Option<String>, // Local relay URL hosted by this coordinator
    pub parent_coordinator_url: Option<String>, // Parent coordinator URL

    // Extensions
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub identity: Option<IdentityInfo>,
    #[serde(default)]
    pub connections: Option<ConnectionsInfo>,
    #[serde(default)]
    pub tls: Option<TlsInfo>,
    #[serde(default)]
    pub relay: Option<RelayInfo>,
}

fn default_status() -> String {
    "unknown".to_string()
}

#[derive(Debug)]
pub struct InfoState {
    pub endpoint_addr_bytes: Vec<u8>,
    pub substrate_id: String,
    pub relay_url: Option<String>,
    pub parent_coordinator_url: Option<String>,
    pub active_connections: Arc<std::sync::atomic::AtomicUsize>,
    pub max_connections: Option<usize>,
    pub tls_cert_path: Option<std::path::PathBuf>,
    pub is_relay_enabled: bool,
    pub registry_client: syneroym_core::dht_registry::RegistryClient,
}

fn get_cert_expiry_days(path: &std::path::Path) -> Option<i64> {
    let cert_data = fs::read(path).ok()?;
    let der = match parse_x509_pem(&cert_data) {
        Ok((_, pem_obj)) => pem_obj.contents,
        Err(_) => cert_data,
    };

    let (_, cert) = X509Certificate::from_der(&der).ok()?;
    let not_after = cert.validity().not_after.timestamp();
    let now = chrono::Utc::now().timestamp();
    let diff_seconds = not_after - now;
    Some(diff_seconds / (24 * 3600))
}

pub async fn get_info(State(state): State<Arc<InfoState>>) -> Json<CoordinatorInfo> {
    let active = state.active_connections.load(std::sync::atomic::Ordering::SeqCst);
    let cap = state.max_connections;

    let tls = if let Some(ref path) = state.tls_cert_path {
        if path.exists() {
            Some(TlsInfo { cert_expiry_days: get_cert_expiry_days(path) })
        } else {
            None
        }
    } else {
        None
    };

    let controller_status = match state.registry_client.lookup(&state.substrate_id, false).await {
        Ok(_) => "verified".to_string(),
        Err(_) => "unverified".to_string(),
    };

    let status = if let Some(max_conn) = cap {
        if active >= max_conn { "at_capacity".to_string() } else { "healthy".to_string() }
    } else {
        "healthy".to_string()
    };

    let info = CoordinatorInfo {
        endpoint_addr_bytes: state.endpoint_addr_bytes.clone(),
        substrate_id: state.substrate_id.clone(),
        relay_url: state.relay_url.clone(),
        parent_coordinator_url: state.parent_coordinator_url.clone(),
        status,
        identity: Some(IdentityInfo { did: state.substrate_id.clone(), controller_status }),
        connections: Some(ConnectionsInfo { active, cap }),
        tls,
        relay: Some(RelayInfo { online: state.is_relay_enabled }),
    };
    Json(info)
}
