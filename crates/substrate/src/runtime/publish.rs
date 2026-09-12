//! Community registry publication and instance certificate expiry checking.

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use iroh::EndpointAddr;
use syneroym_core::{
    dht_registry::{
        DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointMechanism, EndpointType,
        HEARTBEAT_INTERVAL_SECS, RegistryClient, SignedEndpointInfo,
    },
    endpoint_publisher::EndpointPublisher,
    local_registry::EndpointRegistry,
};
use syneroym_identity::Identity;
use tokio::{
    sync::{mpsc, oneshot},
    time,
};
use tracing::{info, warn};

/// One heartbeat pass's worth of work: (re-)register this substrate's own
/// endpoint record, then replay every hosted service's still-verifying
/// record. Split out of `publish_to_community_registry`'s loop so a forced
/// republish (`ControlPlaneService::republish_now`) can run the exact same
/// work as the scheduled heartbeat, not a parallel, drifting copy of it.
pub(super) async fn publish_self_and_hosted(
    service_id: &str,
    endpoint_addr: &EndpointAddr,
    relay_url: Option<String>,
    secret_key: &[u8; 32],
    nickname: Option<String>,
    registry_client: &RegistryClient,
    publisher: &EndpointPublisher,
) -> anyhow::Result<()> {
    let signed_info =
        build_signed_endpoint_info(service_id, endpoint_addr, relay_url, secret_key, nickname)
            .map_err(|e| anyhow::anyhow!("failed to build signed endpoint info: {e}"))?;

    let mut attempts = 0;
    let mut success = false;
    while attempts < 30 {
        if let Err(e) = registry_client.register(&signed_info, false).await {
            warn!("Failed to register endpoint (attempt {}): {}", attempts + 1, e);
            time::sleep(Duration::from_millis(500)).await;
            attempts += 1;
        } else {
            success = true;
            break;
        }
    }

    if success {
        info!(service_id = %service_id, "Successfully registered substrate endpoint");
    } else {
        warn!(
            service_id = %service_id,
            "Exhausted registration retries. Substrate may be unreachable."
        );
    }

    // Hosted services: replay every stored, still-verifying record
    // verbatim. The substrate holds no key that could ever sign one itself
    // (ADR-0020 §3), so this is pure replay, never a rebuild.
    publisher.publish_all_services().await;
    publisher.warn_on_near_expiry_records().await;

    if success {
        Ok(())
    } else {
        Err(anyhow::anyhow!("exhausted registration retries for this substrate's own endpoint"))
    }
}

pub(super) fn publish_to_community_registry(
    service_id: String,
    endpoint_addr: EndpointAddr,
    relay_url: Option<String>,
    secret_key: [u8; 32],
    nickname: Option<String>,
    publisher: Arc<EndpointPublisher>,
    mut force_rx: mpsc::Receiver<oneshot::Sender<anyhow::Result<()>>>,
) {
    tokio::spawn(async move {
        // Reuses the publisher's own client rather than building a second
        // one from the same config: each opens its own pkarr DHT client
        // when the DHT is enabled, so a duplicate is a real (if small) cost,
        // not just noise.
        let registry_client = publisher.registry_client();

        // A forced request that arrived while a pass was already running
        // (or while the pass just triggered by an earlier request was
        // running) waits here for the *next* pass to reply to, rather than
        // the caller's request going unanswered.
        let mut pending_reply: Option<oneshot::Sender<anyhow::Result<()>>> = None;
        loop {
            let result = publish_self_and_hosted(
                &service_id,
                &endpoint_addr,
                relay_url.clone(),
                &secret_key,
                nickname.clone(),
                &registry_client,
                &publisher,
            )
            .await;
            if let Some(reply) = pending_reply.take() {
                let _ = reply.send(result);
            }

            // Sleep until the next heartbeat interval, unless a forced
            // republish request arrives first -- in which case loop back
            // around immediately and run another pass for it.
            tokio::select! {
                () = time::sleep(Duration::from_secs(HEARTBEAT_INTERVAL_SECS)) => {},
                Some(reply) = force_rx.recv() => pending_reply = Some(reply),
            }
        }
    });
}

/// The attended posture's visibility half (ADR-0020 §3): nothing here
/// renews a certificate, only warns before a missed renewal becomes an
/// outage. Runs on the same cadence as the community-registry heartbeat
/// above but as its own sibling loop in `RuntimeServices`'s `select!`,
/// rather than growing `publish_to_community_registry`'s argument list with
/// a registry it has no other reason to hold.
pub(super) async fn instance_cert_expiry_sweep_loop(registry: &EndpointRegistry) -> ! {
    loop {
        warn_on_near_expiry_instance_certs(registry);
        time::sleep(Duration::from_secs(HEARTBEAT_INTERVAL_SECS)).await;
    }
}

/// Warns for any installed instance certificate within 25% of its lifetime
/// of expiring, and returns their `service_id`s. Split out from the sleep
/// loop above -- and returning the warned set rather than only logging it --
/// so it's testable without waiting on a real timer or scraping log output.
pub(super) fn warn_on_near_expiry_instance_certs(registry: &EndpointRegistry) -> Vec<String> {
    let now_secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

    let mut near_expiry = Vec::new();
    for (service_id, cert) in registry.all_instance_certs() {
        if cert.is_near_expiry(now_secs) {
            let remaining_secs = cert.expires_at_secs.saturating_sub(now_secs);
            warn!(
                service_id = %service_id,
                expires_at_secs = cert.expires_at_secs,
                remaining_secs,
                "instance certificate is within 25% of its lifetime of expiring -- renew with                  `roymctl identity certify-instance` before it lapses, which fails the                  handshake closed"
            );
            near_expiry.push(service_id);
        }
    }
    near_expiry
}

pub(super) fn build_signed_endpoint_info(
    service_id: &str,
    endpoint_addr: &EndpointAddr,
    relay_url: Option<String>,
    secret_key: &[u8; 32],
    nickname: Option<String>,
) -> anyhow::Result<SignedEndpointInfo> {
    // Prune direct addresses to keep the serialized PKARR record under the
    // 1000-byte DNS limit
    let pruned_addr = EndpointAddr::new(endpoint_addr.id);
    let endpoint_addr_bytes = serde_json::to_vec(&pruned_addr)
        .map_err(|e| anyhow::anyhow!("Failed to serialize endpoint addr: {e}"))?;

    let not_after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS);

    let info = EndpointInfo {
        service_id: service_id.to_string(),
        substrate_id: service_id.to_string(),
        endpoint_type: EndpointType::Substrate,
        nickname,
        mechanisms: vec![EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url }],
        is_private: false,
        ttl: None,
        not_after,
        generation: 0,
    };

    let identity = Identity::from_bytes(secret_key);
    info.sign(&identity)
}
