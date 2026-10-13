use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use axum::{Json, Router, extract::State, routing};
use chrono::Utc;
use iroh_relay::server::Metrics as RelayServerMetrics;
use syneroym_core::coordinator_info::{
    ConnectionsInfo, CoordinatorInfo, CoordinatorStatus, INFO_PATH, RegistryInfo, RelayInfo,
    TlsInfo,
};
use tokio::{sync::watch, task};
use x509_parser::{
    pem::parse_x509_pem,
    prelude::{FromDer, X509Certificate},
};

#[derive(Debug)]
pub(crate) struct InfoState {
    pub endpoint_addr_bytes: Vec<u8>,
    /// The coordinator's Iroh endpoint id.
    pub substrate_id: String,
    pub relay_url: Option<String>,
    pub parent_coordinator_url: Option<String>,
    pub active_connections: Arc<AtomicUsize>,
    pub max_connections: Option<usize>,
    pub tls_cert_path: Option<PathBuf>,
    /// Counters of the relay server this coordinator runs. `None` when it runs
    /// no relay server.
    pub relay_metrics: Option<Arc<RelayServerMetrics>>,
    pub max_relay_clients: Option<u64>,
    /// The coordinator DID and the result of the last registration. `None`
    /// when the coordinator does not register itself.
    pub registry: Option<(String, watch::Receiver<bool>)>,
}

fn get_cert_expiry_days(path: &Path) -> Option<i64> {
    let cert_data = fs::read(path).ok()?;
    let der = match parse_x509_pem(&cert_data) {
        Ok((_, pem_obj)) => pem_obj.contents,
        Err(_) => cert_data,
    };

    let (_, cert) = X509Certificate::from_der(&der).ok()?;
    let not_after = cert.validity().not_after.timestamp();
    let now = Utc::now().timestamp();
    let diff_seconds = not_after - now;
    Some(diff_seconds / (24 * 3600))
}

/// Open relay connections now. `iroh-relay` has no gauge for this; each
/// client actor counts one accept when it starts and one disconnect when it
/// ends. An endpoint that connects again counts twice until its old
/// connection closes.
pub(crate) fn relay_client_count(metrics: &RelayServerMetrics) -> u64 {
    // Read disconnects first: a connection's disconnect is counted after its
    // accept, so this order does not see a disconnect without its accept.
    // The counters are relaxed atomics, so the order is not guaranteed;
    // `saturating_sub` keeps the result at zero or above.
    let disconnects = metrics.disconnects.get();
    metrics.accepts.get().saturating_sub(disconnects)
}

pub(crate) fn relay_status(clients: Option<u64>, cap: Option<u64>) -> CoordinatorStatus {
    match (clients, cap) {
        (Some(clients), Some(cap)) if clients >= cap => CoordinatorStatus::AtCapacity,
        _ => CoordinatorStatus::Healthy,
    }
}

pub(crate) async fn get_info(State(state): State<Arc<InfoState>>) -> Json<CoordinatorInfo> {
    let relay_clients = state.relay_metrics.as_deref().map(relay_client_count);
    let tls = match &state.tls_cert_path {
        Some(path) => {
            let path = path.clone();
            task::spawn_blocking(move || {
                if path.exists() {
                    Some(TlsInfo { cert_expiry_days: get_cert_expiry_days(&path) })
                } else {
                    None
                }
            })
            .await
            .ok()
            .flatten()
        }
        None => None,
    };

    Json(CoordinatorInfo {
        endpoint_addr_bytes: state.endpoint_addr_bytes.clone(),
        substrate_id: state.substrate_id.clone(),
        relay_url: state.relay_url.clone(),
        parent_coordinator_url: state.parent_coordinator_url.clone(),
        status: relay_status(relay_clients, state.max_relay_clients),
        identity: None,
        connections: Some(ConnectionsInfo {
            active: state.active_connections.load(Ordering::SeqCst),
            cap: state.max_connections,
        }),
        tls,
        relay: Some(RelayInfo {
            online: state.relay_metrics.is_some(),
            clients: relay_clients,
            max_clients: state.max_relay_clients,
        }),
        registry: state.registry.as_ref().map(|(did, registered)| RegistryInfo {
            did: did.clone(),
            registered: *registered.borrow(),
        }),
    })
}

/// The routes of the info server.
pub(crate) fn info_router(state: InfoState) -> Router {
    Router::new().route(INFO_PATH, routing::get(get_info)).with_state(Arc::new(state))
}

#[cfg(test)]
mod tests;
