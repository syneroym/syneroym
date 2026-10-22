//! Helpers for tests that drive a guest's durable outbox across real
//! substrates: the fast queue role, a published guest, and the operator
//! views of its outbox.
//!
//! Skips nothing by itself: a test that needs the `proxy-test` and
//! `greeter` wasm artifacts calls [`artifacts`] and decides.

#![allow(clippy::cognitive_complexity, dead_code)]

use std::{fs, time::Duration};

use ed25519_dalek::VerifyingKey;
use reqwest::Client;
use serde_json::{Value, json};
use syneroym_core::{
    config::AppSandboxRole,
    dht_registry::{EndpointInfo, EndpointMechanism, EndpointType, RegistryClient},
    test_constants,
};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_sdk::SyneroymClient;

use super::SubstrateNode;
pub use super::roym::wait_until;

/// Long enough that the item is still waiting when node B finishes booting
/// again -- a budget that ran out first would dead-letter the call for a
/// reason the test is not about.
pub const DELIVERY_ATTEMPT_BUDGET: u8 = 200;
/// Short enough to exhaust promptly, since exhausting is the point.
pub const DLQ_ATTEMPT_BUDGET: u8 = 3;

/// A queued call must not have to wait out the production ~10-hour budget
/// for a test to see it dead-letter. Deliberately not the real numbers:
/// this scenario needs the *sequence*, and the production window is
/// already pinned by `the_sandbox_role_defaults_give_the_same_ten_hour_window`.
///
/// `max_attempts` is the parameter because the two tests want opposite
/// things from it: the delivery case needs a budget that comfortably
/// outlasts a node reboot (otherwise the item dead-letters while its
/// target is still coming up, and the test would be asserting the wrong
/// outcome for the right reason), and the dead-letter case needs one small
/// enough to exhaust promptly.
pub fn fast_queue_role(max_attempts: u8) -> AppSandboxRole {
    AppSandboxRole {
        queue_tick_secs: 1,
        queue_max_attempts: max_attempts,
        queue_max_backoff_secs: 1,
        // Zero so an abandoned claim is immediately re-claimable: a tick
        // that finds the target still down must not then hide the item
        // from the next tick for two minutes.
        queue_visibility_timeout_secs: 0,
        ..AppSandboxRole::default()
    }
}

/// The `.configure` hook node A boots with: the fast queue role for this
/// test's attempt budget, plus one connect attempt per delivery. The
/// proxy's own retry loop sits *underneath* the outbox here, so leaving it
/// at three multiplies every queued attempt by three call timeouts for
/// nothing -- the outbox is already the retry mechanism.
pub fn configure_queue_node(
    max_attempts: u8,
) -> impl Fn(&mut syneroym_core::config::SubstrateConfig) + Send + Sync + 'static {
    move |config| {
        config.roles.app_sandbox = Some(fast_queue_role(max_attempts));
        config.retry.max_attempts = 1;
    }
}

/// Publishes `service_id`'s endpoint record so another node's proxy can
/// resolve it. Without this the proxy answers `service-not-found`, which is
/// *terminal* -- so an unpublished target never reaches the outbox at all,
/// and the "unreachable" case these tests need would not be the one they
/// mean to exercise.
pub async fn publish_endpoint(
    service_id: &str,
    substrate_id: &str,
    mechanisms: Vec<EndpointMechanism>,
    signer: &Identity,
    registry_url: &str,
) {
    let mechanisms_snapshot = mechanisms.clone();
    let info = EndpointInfo {
        service_id: service_id.to_string(),
        substrate_id: substrate_id.to_string(),
        endpoint_type: EndpointType::Service,
        nickname: None,
        mechanisms,
        is_private: false,
        ttl: None,
        not_after: u64::MAX / 2,
        generation: 0,
        mailbox: None,
    };
    let signed = info.sign(signer).unwrap();
    let res = Client::new()
        .post(format!("{registry_url}/register"))
        .json(&signed)
        .send()
        .await
        .expect("registry register request failed");
    assert!(res.status().is_success(), "registry rejected the record: {:?}", res.text().await);

    // Read it back before returning. A target that does not resolve is a
    // *terminal* failure at the proxy, so a silently-absent record would
    // make these tests exercise a different case than the one they mean
    // to -- and would do it as a confusing "no such service" rather than
    // as a visible setup failure.
    let readback = wait_until(Duration::from_secs(20), || {
        let url = format!("{registry_url}/lookup/{service_id}");
        async move { Client::new().get(&url).send().await.is_ok_and(|r| r.status().is_success()) }
    })
    .await;
    assert!(readback, "the registry never served back the record for {service_id}");

    // The proxy's remote hop needs an Iroh mechanism specifically; a
    // record carrying only WebRTC resolves to nothing and surfaces as the
    // same terminal "no such service".
    assert!(
        mechanisms_snapshot.iter().any(|m| matches!(m, EndpointMechanism::Iroh { .. })),
        "the published record for {service_id} carries no Iroh mechanism: {mechanisms_snapshot:?}"
    );
}

/// Deploys the `proxy-test` guest as `master`'s own DID, with an installed
/// instance certificate.
///
/// The certificate is not optional dressing: `enqueue` refuses a service
/// that holds no unexpired one, because every delivery attempt would
/// otherwise present as anonymous and be refused by the receiver.
pub async fn deploy_guest(node: &mut SubstrateNode, master: &Identity, wasm: Vec<u8>) -> String {
    let service_id = substrate::derive_did_key(&master.public_key());
    // The client's connection may have sat idle through earlier setup long
    // enough for the peer to abandon it; one redial before the real call.
    let identity = match node.substrate_client.instance_identity(&service_id).await {
        Ok(identity) => identity,
        Err(_) => {
            node.substrate_client.shutdown().await.expect("failed to reset stale connection");
            node.substrate_client.connect().await.expect("failed to reconnect");
            node.substrate_client
                .instance_identity(&service_id)
                .await
                .expect("instance identity failed even after reconnect")
        }
    };
    let pubkey_bytes: [u8; 32] = hex::decode(&identity.pubkey_hex)
        .expect("instance pubkey is not hex")
        .try_into()
        .expect("instance pubkey is not 32 bytes");
    let instance_pubkey = VerifyingKey::from_bytes(&pubkey_bytes).unwrap();
    let cert = DelegationCertificate::issue(
        master,
        instance_pubkey,
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();

    node.substrate_client
        .deploy_svc_wasm(
            service_id.clone(),
            vec![test_constants::PROXY_TEST_DRIVER_INTERFACE.to_string()],
            wasm,
            syneroym_sdk::Publication::Private,
            Some(cert),
        )
        .await
        .expect("guest deploy failed");

    // The receiving node re-verifies the guest's instance certificate on
    // every delivery, and cannot do that without the master's anchor. An
    // unverifiable delegation is rejected harder than no delegation at
    // all, so without this every attempt fails at the handshake -- which
    // is a *transport* failure, so the item never dead-letters and never
    // lands either: it simply retries forever.
    RegistryClient::new(false, Some(node.registry_url().to_string()))
        .publish_master_anchor(&service_id, vec![], None, master, true)
        .await
        .expect("failed to publish the caller master's anchor");

    // Published as well as deployed: the test drives the guest through an
    // ordinary client, which resolves it through the registry like any
    // other caller would.
    let mechanisms =
        node.substrate_client.lookup().await.expect("node lookup failed").info.mechanisms;
    publish_endpoint(&service_id, node.did(), mechanisms, master, node.registry_url()).await;
    service_id
}

/// Drives the guest's own `enqueue-peer` export -- real guest code calling
/// `syneroym:proxy/proxy::enqueue`, not a Rust-level fake.
pub async fn guest_enqueue(
    node: &SubstrateNode,
    guest_service_id: &str,
    target_did: &str,
    idempotency_key: &str,
) -> Result<Value, String> {
    let mut client = SyneroymClient::new_with_identity(
        guest_service_id.to_string(),
        node.registry_url().to_string(),
        Identity::generate().unwrap(),
    )
    .with_registry_dht(false);
    client.connect().await.map_err(|e| format!("connect failed: {e}"))?;
    let outcome = client
        .request(
            test_constants::PROXY_TEST_DRIVER_INTERFACE,
            "enqueue-peer",
            json!({
                "service": target_did,
                "interface": test_constants::GREETER_INTERFACE_NAME,
                "method": "greet",
                "params": "[\"queued\"]",
                "target-kind": "service",
                "idempotency-key": idempotency_key,
            }),
        )
        .await
        .map(|r| r.result)
        .map_err(|e| e.to_string());
    client.shutdown().await.ok();
    outcome
}

pub async fn outbox_keys(node: &SubstrateNode, service_id: &str) -> Vec<String> {
    node.substrate_client
        .proxy_outbox(service_id.to_string())
        .await
        .expect("proxy-outbox failed")
        .into_iter()
        .map(|i| i.idempotency_key)
        .collect()
}

pub async fn dead_letter_keys(node: &SubstrateNode, service_id: &str) -> Vec<String> {
    node.substrate_client
        .proxy_dead_letters(service_id.to_string())
        .await
        .expect("proxy-dead-letters failed")
        .into_iter()
        .map(|i| i.idempotency_key)
        .collect()
}

pub fn artifacts() -> Option<(Vec<u8>, Vec<u8>)> {
    let proxy = fs::read(test_constants::proxy_test_wasm_path()).ok()?;
    let greeter = fs::read(test_constants::greeter_wasm_path()).ok()?;
    Some((proxy, greeter))
}

/// How many guest components this process has instantiated so far, across
/// every node it runs.
pub fn wasm_instantiations() -> u64 {
    syneroym_observability::MemoryRecorder::global()
        .expect("global MemoryRecorder must be installed by the substrate under test")
        .snapshot()
        .counters
        .get("substrate.wasm.instantiations_total")
        .copied()
        .unwrap_or(0)
}
