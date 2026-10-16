use std::sync::{Arc, atomic::AtomicUsize};

use axum::extract::State;
use iroh_relay::server::Metrics as RelayServerMetrics;
use syneroym_core::coordinator_info::CoordinatorStatus;
use tokio::sync::watch;

use super::*;

#[test]
fn client_count_is_accepts_minus_disconnects() {
    let metrics = RelayServerMetrics::default();
    metrics.accepts.inc();
    metrics.accepts.inc();
    metrics.accepts.inc();
    metrics.disconnects.inc();
    assert_eq!(relay_client_count(&metrics), 2);

    metrics.disconnects.inc();
    metrics.disconnects.inc();
    metrics.disconnects.inc();
    assert_eq!(relay_client_count(&metrics), 0);
}

#[test]
fn status_is_at_capacity_only_at_or_above_the_cap() {
    assert_eq!(relay_status(None, None), CoordinatorStatus::Healthy);
    assert_eq!(relay_status(Some(5), None), CoordinatorStatus::Healthy);
    assert_eq!(relay_status(None, Some(1)), CoordinatorStatus::Healthy);
    assert_eq!(relay_status(Some(1), Some(2)), CoordinatorStatus::Healthy);
    assert_eq!(relay_status(Some(2), Some(2)), CoordinatorStatus::AtCapacity);
    assert_eq!(relay_status(Some(3), Some(2)), CoordinatorStatus::AtCapacity);
}

#[tokio::test]
async fn info_reads_state_without_network() {
    let metrics = Arc::new(RelayServerMetrics::default());
    metrics.accepts.inc();
    metrics.accepts.inc();

    let (_tx, rx) = watch::channel(true);
    let state = InfoState {
        endpoint_addr_bytes: vec![1, 2, 3],
        substrate_id: "test-substrate".to_string(),
        relay_url: Some("https://relay.test".to_string()),
        parent_coordinator_url: None,
        active_connections: Arc::new(AtomicUsize::new(0)),
        max_connections: None,
        tls_cert_path: None,
        relay_metrics: Some(metrics),
        max_relay_clients: Some(2),
        identity: IdentityInfo {
            did: "did:key:test-host".to_string(),
            controller_status: "none".to_string(),
        },
        registry: Some(("did:key:test".to_string(), rx)),
    };

    let Json(info) = get_info(State(Arc::new(state))).await;
    assert_eq!(info.status, CoordinatorStatus::AtCapacity);
    let relay = info.relay.expect("relay should be present");
    assert!(relay.online);
    assert_eq!(relay.clients, Some(2));
    assert_eq!(relay.max_clients, Some(2));
    assert_eq!(info.identity.as_ref().unwrap().did, "did:key:test-host");
    assert_eq!(info.identity.as_ref().unwrap().controller_status, "none");
    let reg = info.registry.expect("registry should be present");
    assert_eq!(reg.did, "did:key:test");
    assert!(reg.registered);
}
