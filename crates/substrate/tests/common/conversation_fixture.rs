//! The dual-build-fixture guest as a conversation peer: deploy it under a
//! master identity, publish where the other node's proxy resolves it, and
//! drive its `test-driver::run` export. Shared by every cross-node
//! conversation test, so each boots peers the same way.

use std::{fs, time::Duration};

use ed25519_dalek::VerifyingKey;
use reqwest::Client;
use serde_json::{Value, json};
use syneroym_core::{
    dht_registry::{EndpointInfo, EndpointMechanism, EndpointType, RegistryClient},
    test_constants,
};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_sdk::SyneroymClient;

use super::{SubstrateNode, roym::wait_until};

/// Mirrors `syneroym_test_dual_build_fixture::native::FIXTURE_INTERFACE`
/// (`wit/world.wit`'s `test-driver` export) without pulling in that crate
/// as a dependency -- these tests drive the deployed WASM component purely
/// over the wire, the same way `execute_wasm_json` reaches any other
/// guest export, and never link the native shim.
pub const FIXTURE_INTERFACE: &str = "syneroym-test:dual-build-fixture/test-driver@0.1.0";

pub fn fixture_wasm() -> Option<Vec<u8>> {
    fs::read(test_constants::dual_build_fixture_wasm_path()).ok()
}

/// Publishes `service_id`'s endpoint record so the other node's proxy can
/// resolve it -- copied verbatim from `proxy_outbox_e2e.rs`.
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

/// How a fixture service is deployed. `NoCertificate` exists to test a peer
/// that is missing something an honest one has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deploy {
    /// With an instance certificate: it can present an identity to a peer.
    Certified,
    /// No instance certificate, so it cannot.
    NoCertificate,
}

/// Deploys the dual-build-fixture guest as `master`'s own DID. An
/// uncertified service is refused on every send and deliver attempt.
/// Mirrors
/// `proxy_outbox_e2e.rs`'s `deploy_guest`.
pub async fn deploy_fixture(
    node: &mut SubstrateNode,
    master: &Identity,
    wasm: Vec<u8>,
    mode: Deploy,
) -> String {
    let service_id = substrate::derive_did_key(&master.public_key());
    let cert = match mode {
        Deploy::NoCertificate => None,
        Deploy::Certified => Some(instance_certificate(node, master, &service_id).await),
    };

    node.substrate_client
        .deploy_svc_wasm(
            service_id.clone(),
            vec![FIXTURE_INTERFACE.to_string()],
            wasm,
            syneroym_sdk::Publication::Private,
            cert,
        )
        .await
        .expect("fixture deploy failed");

    publish_master_anchor(&service_id, master, node.registry_url()).await;

    // Published as well as deployed: every test call reaches the fixture
    // through an ordinary client, which resolves it through the registry
    // like any other caller would.
    let mechanisms =
        node.substrate_client.lookup().await.expect("node lookup failed").info.mechanisms;
    publish_endpoint(&service_id, node.did(), mechanisms, master, node.registry_url()).await;
    service_id
}

/// The instance certificate `master` issues to the instance key the node
/// holds for `service_id`.
async fn instance_certificate(
    node: &mut SubstrateNode,
    master: &Identity,
    service_id: &str,
) -> DelegationCertificate {
    // A client's connection is dialed during boot and can sit idle long
    // enough for the peer to abandon that path; one explicit redial
    // recovers it (`retry.rs`'s `call_with_reconnect!`, which a shared
    // module cannot use without every test binary declaring it).
    let identity = match node.substrate_client.instance_identity(service_id).await {
        Ok(identity) => identity,
        Err(_) => {
            node.substrate_client.shutdown().await.expect("failed to reset stale connection");
            node.substrate_client.connect().await.expect("failed to reconnect");
            node.substrate_client
                .instance_identity(service_id)
                .await
                .expect("instance_identity failed even after reconnect")
        }
    };
    let pubkey_bytes: [u8; 32] = hex::decode(&identity.pubkey_hex)
        .expect("instance pubkey is not hex")
        .try_into()
        .expect("instance pubkey is not 32 bytes");
    let instance_pubkey = VerifyingKey::from_bytes(&pubkey_bytes).unwrap();
    DelegationCertificate::issue(master, instance_pubkey, 3600, SCOPE_SERVICE_INSTANCE.to_string())
        .unwrap()
}

/// The sender's master anchor must be resolvable wherever the *receiving*
/// node's registry lives, or every delivery attempt fails the handshake
/// (a transport failure, not a delivery outcome) rather than landing or
/// dead-lettering.
pub async fn publish_master_anchor(service_id: &str, master: &Identity, registry_url: &str) {
    RegistryClient::new(false, Some(registry_url.to_string()))
        .publish_master_anchor(service_id, vec![], None, master, true)
        .await
        .expect("failed to publish the master anchor");
}

/// Copies `node`'s own signed endpoint record into another registry, so a
/// node that resolves through that registry can dial it.
pub async fn publish_node_record(node: &SubstrateNode, registry_url: &str) {
    let record = node.substrate_client.lookup().await.expect("node lookup failed");
    let res = Client::new()
        .post(format!("{registry_url}/register"))
        .json(&record)
        .send()
        .await
        .expect("registry register request failed");
    assert!(res.status().is_success(), "registry rejected the node record: {:?}", res.text().await);
}

/// Drives the fixture's own `test-driver::run` export -- real guest code
/// calling `syneroym:conversation`, not a Rust-level fake.
pub async fn fixture_run(node: &SubstrateNode, service_id: &str, request: &Value) -> Value {
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

/// The `ok` payload of a fixture reply; panics with the reply if the
/// fixture answered with an error instead.
fn ok_of(reply: Value, what: &str) -> Value {
    assert!(reply.get("ok").is_some(), "{what} did not succeed: {reply}");
    reply["ok"].clone()
}

/// Opens a direct conversation to `peer_address`; returns its id.
pub async fn open_conversation(
    node: &SubstrateNode,
    service_id: &str,
    peer_address: &str,
) -> String {
    let reply = fixture_run(
        node,
        service_id,
        &json!({"op": "open-conversation", "peer_address": peer_address}),
    )
    .await;
    ok_of(reply, "open-conversation")["conversation"].as_str().unwrap().to_string()
}

/// One send, expecting it to be accepted; returns the message id.
pub async fn send_message(
    node: &SubstrateNode,
    service_id: &str,
    conversation: &str,
    body: &str,
) -> String {
    let reply = try_send_message(node, service_id, conversation, body).await;
    ok_of(reply, "send-message")["message"].as_str().unwrap().to_string()
}

/// One send, returning the fixture's raw reply so a test can read a refusal
/// (`quota-exceeded`) as well as an accepted id.
pub async fn try_send_message(
    node: &SubstrateNode,
    service_id: &str,
    conversation: &str,
    body: &str,
) -> Value {
    fixture_run(
        node,
        service_id,
        &json!({"op": "send-message", "conversation": conversation, "body": body}),
    )
    .await
}

/// Every message the host holds for `conversation`, newest window first.
pub async fn history_messages(
    node: &SubstrateNode,
    service_id: &str,
    conversation: &str,
) -> Vec<Value> {
    let reply = fixture_run(
        node,
        service_id,
        &json!({"op": "read-history", "conversation": conversation, "limit": 50}),
    )
    .await;
    ok_of(reply, "read-history")["messages"].as_array().cloned().unwrap_or_default()
}

/// The one history row for `message`, if the host has it.
pub async fn message_row(
    node: &SubstrateNode,
    service_id: &str,
    conversation: &str,
    message: &str,
) -> Option<Value> {
    history_messages(node, service_id, conversation).await.into_iter().find(|m| m["id"] == message)
}

/// Polls until `message`'s delivery state is `want`.
pub async fn wait_for_state(
    node: &SubstrateNode,
    service_id: &str,
    message: &str,
    want: &str,
    budget: Duration,
) -> bool {
    wait_until(budget, || async {
        let reply =
            fixture_run(node, service_id, &json!({"op": "delivery-status", "message": message}))
                .await;
        reply["ok"]["state"] == want
    })
    .await
}
