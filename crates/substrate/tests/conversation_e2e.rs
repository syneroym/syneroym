#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Durable 1:1 messaging, end to end across two real `syneroym-substrate`
//! instances: A messages B while B is offline, the message stays `pending`
//! in A's own outbox; A restarts and the same item is still there, not
//! duplicated and not lost; B comes up and the message is delivered,
//! verified, and readable through the host interface; and no durable
//! content ever crosses `syneroym:messaging` (ADR-0013 §6).
//!
//! Both nodes come from `common::SubstrateNode`. Node A hosts the shared
//! registry and restarts mid-test, so its builder is captured and reused to
//! reboot on the same ports; Node B resolves through that registry.
//!
//! Skips when the dual-build-fixture wasm artifact is absent
//! (`mise run build:test-components`, or `cargo component build --release
//! --target wasm32-wasip2 -p syneroym-test-dual-build-fixture`).

use std::{
    fs,
    time::{Duration, Instant},
};

use common::SubstrateNode;
use ed25519_dalek::VerifyingKey;
use reqwest::Client;
use rustls::crypto::ring;
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
use tokio::time;

mod common;

#[path = "common/retry.rs"]
mod retry;

/// Mirrors `syneroym_test_dual_build_fixture::native::FIXTURE_INTERFACE`
/// (`wit/world.wit`'s `test-driver` export) without pulling in that crate
/// as a dependency -- this file drives the deployed WASM component purely
/// over the wire, the same way `execute_wasm_json` reaches any other
/// guest export, and never links the native shim.
const FIXTURE_INTERFACE: &str = "syneroym-test:dual-build-fixture/test-driver@0.1.0";

/// A conversation delivery attempt must not wait out the production
/// ~10-hour attempt budget for this test to see it stay `pending`; the
/// default `conversation_max_pending_age_secs` (30 days) is left alone --
/// this test never lets a delivery attempt actually fail, only stay
/// `pending` while the peer does not yet exist.
fn fast_conversation_role() -> AppSandboxRole {
    AppSandboxRole { conversation_tick_secs: 1, ..AppSandboxRole::default() }
}

/// Publishes `service_id`'s endpoint record so the other node's proxy can
/// resolve it -- copied verbatim from `proxy_outbox_e2e.rs`.
async fn publish_endpoint(
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
    };
    let signed = info.sign(signer).unwrap();
    let res = Client::new()
        .post(format!("{registry_url}/register"))
        .json(&signed)
        .send()
        .await
        .expect("registry register request failed");
    assert!(res.status().is_success(), "registry rejected the record: {:?}", res.text().await);

    let readback = wait_until(Duration::from_secs(20), || {
        let url = format!("{registry_url}/lookup/{service_id}");
        async move { Client::new().get(&url).send().await.is_ok_and(|r| r.status().is_success()) }
    })
    .await;
    assert!(readback, "the registry never served back the record for {service_id}");

    assert!(
        mechanisms_snapshot.iter().any(|m| matches!(m, EndpointMechanism::Iroh { .. })),
        "the published record for {service_id} carries no Iroh mechanism: {mechanisms_snapshot:?}"
    );
}

/// Deploys the dual-build-fixture guest as `master`'s own DID, with an
/// installed instance certificate — uncertified services are refused
/// on every send/deliver attempt. Mirrors `proxy_outbox_e2e.rs`'s
/// `deploy_guest`.
async fn deploy_fixture(node: &mut SubstrateNode, master: &Identity, wasm: Vec<u8>) -> String {
    let service_id = substrate::derive_did_key(&master.public_key());
    let identity = crate::call_with_reconnect!(
        node.substrate_client,
        node.substrate_client.instance_identity(&service_id).await
    );
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
            vec![FIXTURE_INTERFACE.to_string()],
            wasm,
            syneroym_sdk::Publication::Private,
            Some(cert),
        )
        .await
        .expect("fixture deploy failed");

    publish_master_anchor(&service_id, master, node.registry_url()).await;

    // Published as well as deployed: every test call in this file reaches
    // the fixture through an ordinary client, which resolves it through
    // the registry like any other caller would.
    let mechanisms =
        node.substrate_client.lookup().await.expect("node lookup failed").info.mechanisms;
    publish_endpoint(&service_id, node.did(), mechanisms, master, node.registry_url()).await;
    service_id
}

/// The sender's master anchor must be resolvable wherever the *receiving*
/// node's registry lives, or every delivery attempt fails the handshake
/// (a transport failure, not a delivery outcome) rather than landing or
/// dead-lettering.
async fn publish_master_anchor(service_id: &str, master: &Identity, registry_url: &str) {
    RegistryClient::new(false, Some(registry_url.to_string()))
        .publish_master_anchor(service_id, vec![], None, master, true)
        .await
        .expect("failed to publish the master anchor");
}

/// Drives the fixture's own `test-driver::run` export -- real guest code
/// calling `syneroym:conversation`, not a Rust-level fake.
async fn fixture_run(node: &SubstrateNode, service_id: &str, request: &Value) -> Value {
    let mut client = SyneroymClient::new_with_identity(
        service_id.to_string(),
        node.registry_url().to_string(),
        Identity::generate().unwrap(),
    )
    .with_registry_dht(false);
    client.connect().await.expect("connect failed");
    let response = client
        .request(FIXTURE_INTERFACE, "run", json!([request.to_string()]))
        .await
        .expect("run request failed");
    client.shutdown().await.ok();
    let payload: Value = response.result;
    let raw = payload.as_str().expect("test-driver::run must return a JSON string");
    serde_json::from_str(raw).expect("fixture response is not valid JSON")
}

async fn wait_until<F, Fut>(budget: Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if check().await {
            return true;
        }
        time::sleep(Duration::from_millis(300)).await;
    }
    false
}

fn fixture_wasm() -> Option<Vec<u8>> {
    fs::read(test_constants::dual_build_fixture_wasm_path()).ok()
}

/// The reference scenario's steps 6-8, plus row 6: A sends to B while B
/// does not exist yet (stronger than merely offline) -- stays `pending`,
/// never `delivered`. A restarts; the same outbox item survives, not
/// duplicated. B then comes up; the message is delivered, verified on
/// arrival, and no durable content ever reached the pub/sub broker.
#[tokio::test]
async fn a_message_survives_a_restart_and_delivers_once_the_peer_exists() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    let wasm = fixture_wasm().expect("dual-build-fixture wasm artifact not built");

    // Node A hosts the shared registry, so its own restart wipes it --
    // deliberately part of what this test proves survives. Its builder is
    // captured and reused for the reboot so the rebooted node keeps the
    // same ports, which node B still resolves the registry through.
    let a_dir = tempfile::tempdir().unwrap();
    let owner = Identity::generate().unwrap();
    let node_a_builder = SubstrateNode::builder()
        .owner(&owner)
        .base_path(a_dir.path())
        .inject_kek_bytes([0xcd; 32])
        .configure(|c| c.roles.app_sandbox = Some(fast_conversation_role()));
    let mut node_a = node_a_builder.clone().boot().await;
    let shared_registry = node_a.registry_url().to_string();

    let sender_master = Identity::generate().unwrap();
    let sender_did = deploy_fixture(&mut node_a, &sender_master, wasm.clone()).await;

    // The peer's identity is deterministic from its master key, so it can
    // be named before node B ever boots -- this is the "recipient never
    // reachable" case (failure-matrix row 5), not merely "offline".
    let peer_master = Identity::generate().unwrap();
    let peer_did = substrate::derive_did_key(&peer_master.public_key());

    let conv_response = fixture_run(
        &node_a,
        &sender_did,
        &json!({"op": "open-conversation", "peer_address": peer_did}),
    )
    .await;
    let conversation_id = conv_response["ok"]["conversation"]
        .as_str()
        .expect("open-conversation must return an id")
        .to_string();

    let send_response = fixture_run(
        &node_a,
        &sender_did,
        &json!({"op": "send-message", "conversation": conversation_id, "body": "hello from A"}),
    )
    .await;
    let message_id = send_response["ok"]["message"]
        .as_str()
        .expect("send-message must return an id")
        .to_string();

    // Row 3/row 5: the peer does not exist, so several consecutive polls
    // must all see `pending`, never `delivered`.
    for _ in 0..3 {
        let status = fixture_run(
            &node_a,
            &sender_did,
            &json!({"op": "delivery-status", "message": message_id}),
        )
        .await;
        assert_eq!(
            status["ok"]["state"], "pending",
            "a message to a peer that does not exist must never read as delivered"
        );
        time::sleep(Duration::from_millis(500)).await;
    }
    let outbox_before = fixture_run(&node_a, &sender_did, &json!({"op": "read-outbox"})).await;
    let entries_before = outbox_before["ok"]["outbox"].as_array().unwrap();
    assert_eq!(entries_before.len(), 1, "exactly one outbox row before the restart");
    assert_eq!(entries_before[0]["id"], message_id);

    // Row 4: restart the sending substrate with the message still pending.
    node_a.teardown().await;
    node_a = node_a_builder.boot().await;
    // A substrate does not bring its own deployed services back up by
    // itself (`proxy_outbox_e2e.rs`'s own precedent) -- redeploy under the
    // same identity, which also republishes the master anchor and the
    // endpoint record the in-memory registry (hosted by A itself) lost.
    // The conversation store itself lives in `app_local_data_dir`, which
    // *does* survive the restart (same `a_dir`), so this proves the
    // outbox's own persistence, not the deploy catalog's.
    let redeployed_sender_did = deploy_fixture(&mut node_a, &sender_master, wasm.clone()).await;
    assert_eq!(redeployed_sender_did, sender_did, "redeploying must not change the service id");

    let outbox_after = fixture_run(&node_a, &sender_did, &json!({"op": "read-outbox"})).await;
    let entries_after = outbox_after["ok"]["outbox"].as_array().unwrap();
    assert_eq!(
        entries_after.len(),
        1,
        "the restart must leave exactly the same one item, not zero and not two"
    );
    assert_eq!(entries_after[0]["id"], message_id, "the same message id, not a new one");
    assert_eq!(
        entries_after[0]["state"], "pending",
        "still pending, not reset and not double-sent"
    );

    // The peer now comes up.
    let mut node_b = SubstrateNode::builder()
        .owner(&owner)
        .shared_registry(&shared_registry)
        .inject_kek_bytes([0xcd; 32])
        .boot()
        .await;
    let receiver_did = deploy_fixture(&mut node_b, &peer_master, wasm).await;
    assert_eq!(
        receiver_did, peer_did,
        "the deployed service id must be the one A already addressed"
    );
    let node_b_mechanisms =
        node_b.substrate_client.lookup().await.expect("node B lookup failed").info.mechanisms;
    publish_endpoint(&peer_did, node_b.did(), node_b_mechanisms, &peer_master, &shared_registry)
        .await;

    // Delivery resumes on A's next tick.
    let delivered = wait_until(Duration::from_secs(30), || {
        let node_a = &node_a;
        let sender_did = sender_did.clone();
        let message_id = message_id.clone();
        async move {
            let status = fixture_run(
                node_a,
                &sender_did,
                &json!({"op": "delivery-status", "message": message_id}),
            )
            .await;
            status["ok"]["state"] == "delivered"
        }
    })
    .await;
    assert!(delivered, "the message must be delivered once the peer exists and resolves");

    // B's own history: verified on arrival, state delivered, through the
    // host interface -- not inferred from A's own bookkeeping.
    let history = fixture_run(
        &node_b,
        &receiver_did,
        &json!({"op": "read-history", "conversation": conversation_id, "limit": 10}),
    )
    .await;
    let messages = history["ok"]["messages"].as_array().unwrap();
    let received = messages
        .iter()
        .find(|m| m["id"] == message_id)
        .unwrap_or_else(|| panic!("delivered message not found in B's own history: {history}"));
    assert_eq!(received["verified"], true, "a validly signed cross-node delivery must be verified");
    assert_eq!(received["state"], "delivered");
    assert_eq!(received["body"], "hello from A");

    // The app's own on-message export was called (host -> app), read back
    // through data-layer, not in-process state.
    let inbox =
        fixture_run(&node_b, &receiver_did, &json!({"op": "read-conversation-inbox"})).await;
    let inbox_entries = inbox["ok"]["entries"].as_array().unwrap();
    assert!(
        inbox_entries.iter().any(|e| e["id"] == message_id),
        "on-message must have notified the app on B, got {inbox}"
    );

    // Row 6 (ADR-0013 §6): durable content never traverses
    // `syneroym:messaging` -- the pub/sub broker inbox stays empty on both
    // ends, asserted against the broker's own traffic, not by inspection.
    let a_broker_inbox = fixture_run(&node_a, &sender_did, &json!({"op": "read-inbox"})).await;
    assert_eq!(
        a_broker_inbox["ok"]["entries"].as_array().unwrap().len(),
        0,
        "no durable content may traverse the pub/sub broker on the sending side"
    );
    let b_broker_inbox = fixture_run(&node_b, &receiver_did, &json!({"op": "read-inbox"})).await;
    assert_eq!(
        b_broker_inbox["ok"]["entries"].as_array().unwrap().len(),
        0,
        "no durable content may traverse the pub/sub broker on the receiving side"
    );

    node_b.teardown().await;
    node_a.teardown().await;
}
