#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Cross-node conversation cases no other test drives, each between real
//! `syneroym-substrate` instances: a lost delivery acknowledgement, a
//! forged author, a re-presented signing key, the proxy's capability gate
//! for the `conversation` interface, prekey rate limiting, per-conversation
//! quota isolation, the clock-skew check, and a send with no instance
//! certificate.
//!
//! The lost-ack, forged-author and skewed-timestamp cases need a peer that
//! misbehaves on cue, which no honest node does. They use the one-shot
//! hooks in `syneroym_conversation::test_support`, keyed by the sending
//! service id because every node in a test runs inside this one process.
//!
//! Node A hosts the registry; every other node resolves through it.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::{
    SubstrateNode,
    conversation_fixture::{
        Deploy, deploy_fixture, fixture_run, fixture_wasm, history_messages, message_row,
        open_conversation, publish_endpoint, publish_master_anchor, publish_node_record,
        send_message, try_send_message, wait_for_state,
    },
    roym::fast_conversation_role,
};
use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_conversation::{
    ids::derive_conversation_id,
    test_support::{SendOverride, drop_ack_pending, drop_next_ack, override_next_send},
};
use syneroym_core::config::AppSandboxRole;
use syneroym_identity::{Identity, substrate};
use syneroym_sdk::SyneroymClient;

mod common;

const DELIVER_BUDGET: Duration = Duration::from_secs(30);
const FAIL_BUDGET: Duration = Duration::from_secs(30);
const YEAR_MS: i64 = 365 * 24 * 3600 * 1000;

/// One conversation node with the fixture guest deployed on it.
struct Peer {
    node: SubstrateNode,
    master: Identity,
    did: String,
}

impl Peer {
    /// Boots a node with a fast delivery tick and deploys the fixture as a
    /// certified service. `shared_registry` is the first node's registry.
    async fn boot(shared_registry: Option<&str>, role: AppSandboxRole, wasm: &[u8]) -> Self {
        Self::boot_as(Identity::generate().unwrap(), shared_registry, role, wasm, Deploy::Certified)
            .await
    }

    async fn boot_as(
        master: Identity,
        shared_registry: Option<&str>,
        role: AppSandboxRole,
        wasm: &[u8],
        mode: Deploy,
    ) -> Self {
        let mut builder = SubstrateNode::builder()
            .owner(&Identity::generate().unwrap())
            .inject_kek_bytes([0xcd; 32])
            .configure(move |c| c.roles.app_sandbox = Some(role.clone()));
        if let Some(url) = shared_registry {
            builder = builder.shared_registry(url);
        }
        let mut node = builder.boot().await;
        let did = deploy_fixture(&mut node, &master, wasm.to_vec(), mode).await;
        Self { node, master, did }
    }

    fn registry(&self) -> &str {
        self.node.registry_url()
    }

    /// This peer's own view of its conversation with `other`.
    fn conversation_with(&self, other: &Peer) -> String {
        derive_conversation_id(&self.did, &other.did)
    }

    /// Makes `other` resolvable through this peer's own registry, for a peer
    /// that does not share the others' registry.
    async fn introduce(&self, other: &Peer) {
        let mechanisms = other.node.substrate_client.lookup().await.unwrap().info.mechanisms;
        publish_endpoint(&other.did, other.node.did(), mechanisms, &other.master, self.registry())
            .await;
        publish_master_anchor(&other.did, &other.master, self.registry()).await;
        publish_node_record(&other.node, self.registry()).await;
    }

    async fn history_with(&self, other: &Peer) -> Vec<Value> {
        history_messages(&self.node, &self.did, &self.conversation_with(other)).await
    }

    async fn teardown(self) {
        self.node.teardown().await;
    }
}

fn default_role() -> AppSandboxRole {
    fast_conversation_role(AppSandboxRole::default().conversation_max_pending_age_secs)
}

fn wasm() -> Vec<u8> {
    fixture_wasm().expect("dual-build-fixture wasm artifact not built")
}

fn bodies(messages: &[Value]) -> Vec<String> {
    let mut b: Vec<String> =
        messages.iter().map(|m| m["body"].as_str().unwrap().to_string()).collect();
    b.sort();
    b
}

/// A message, sent and waited on until the peer holds it.
async fn deliver(from: &Peer, to: &Peer, conv: &str, body: &str) -> String {
    let id = send_message(&from.node, &from.did, conv, body).await;
    let delivered = wait_for_state(&from.node, &from.did, &id, "delivered", DELIVER_BUDGET).await;
    let row = message_row(&from.node, &from.did, conv, &id).await;
    assert!(delivered, "'{body}' must reach {}; sender's row: {row:?}", to.did);
    id
}

/// A lost acknowledgement makes the sender retry a message the peer already
/// stored. The retry must end `delivered`, the peer must hold exactly one
/// copy, and the *next* message must get through -- for the first message
/// on a new session as well as for a later one on an existing session. After
/// the first lost ack the peer also answers *before* the sender writes
/// again: the peer replies on the session it opened, so the sender must
/// still hold that same session.
#[tokio::test]
async fn a_lost_ack_is_retried_and_stored_once_for_a_new_and_an_existing_session() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    let wasm = wasm();
    let a = Peer::boot(None, default_role(), &wasm).await;
    let b = Peer::boot(Some(a.registry()), default_role(), &wasm).await;
    let conv = open_conversation(&a.node, &a.did, &b.did).await;

    // (a) The ack for the very first message, which creates the session.
    drop_next_ack(&a.did);
    deliver(&a, &b, &conv, "first").await;
    assert!(!drop_ack_pending(&a.did), "the first ack was really dropped");
    assert_eq!(bodies(&b.history_with(&a).await), ["first"], "one copy, not two");
    deliver(&b, &a, &b.conversation_with(&a), "reply, before A writes again").await;
    deliver(&a, &b, &conv, "second, after a lost first ack").await;

    // (b) The ack for a later message on the session that now exists.
    drop_next_ack(&a.did);
    deliver(&a, &b, &conv, "third").await;
    assert!(!drop_ack_pending(&a.did), "the later ack was really dropped");
    deliver(&a, &b, &conv, "fourth, after a lost later ack").await;

    assert_eq!(
        bodies(&b.history_with(&a).await),
        [
            "first",
            "fourth, after a lost later ack",
            "reply, before A writes again",
            "second, after a lost first ack",
            "third"
        ],
        "every message is held exactly once"
    );
    b.teardown().await;
    a.teardown().await;
}

/// Node C delivers to B an envelope whose author claims A, under the
/// conversation id B derives for A. The signature (C's own key) and the
/// conversation id both check out, so only the author check stands between
/// the message and B's history with A. B must refuse it there, store
/// nothing for it, and C's own item must end `failed`. C's honest messages
/// before and after are unaffected.
#[tokio::test]
async fn a_delivery_claiming_another_nodes_authorship_is_refused_and_stores_nothing() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    let wasm = wasm();
    let a = Peer::boot(None, default_role(), &wasm).await;
    let b = Peer::boot(Some(a.registry()), default_role(), &wasm).await;
    let c = Peer::boot(Some(a.registry()), default_role(), &wasm).await;
    let conv = open_conversation(&c.node, &c.did, &b.did).await;

    deliver(&c, &b, &conv, "honest before").await;
    override_next_send(
        &c.did,
        SendOverride {
            author: Some(a.did.clone()),
            conversation_id: Some(derive_conversation_id(&b.did, &a.did)),
            ..SendOverride::default()
        },
    );
    let forged = send_message(&c.node, &c.did, &conv, "claims to be from A").await;
    assert!(
        wait_for_state(&c.node, &c.did, &forged, "failed", FAIL_BUDGET).await,
        "the forged send must settle failed on C"
    );
    deliver(&c, &b, &conv, "honest after").await;

    assert_eq!(bodies(&b.history_with(&c).await), ["honest after", "honest before"]);
    assert!(b.history_with(&a).await.is_empty(), "nothing was stored under A's name");
    for p in [c, b, a] {
        p.teardown().await;
    }
}

/// The same service address answering from a second node has a different
/// instance key. B pinned the first key for that address, so the second
/// node's delivery is refused and B's pin is unchanged.
#[tokio::test]
async fn a_different_signing_key_for_a_pinned_address_is_refused_and_the_pin_holds() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    let wasm = wasm();
    let a = Peer::boot(None, default_role(), &wasm).await;
    let b = Peer::boot(Some(a.registry()), default_role(), &wasm).await;
    let conv = open_conversation(&a.node, &a.did, &b.did).await;
    deliver(&a, &b, &conv, "from the real A").await;

    // The second node keeps its own registry, so the fixture is reached on
    // *it* and not on A, and B is introduced into that registry by hand.
    let impostor = Peer::boot_as(
        Identity::from_bytes(&a.master.to_bytes()),
        None,
        default_role(),
        &wasm,
        Deploy::Certified,
    )
    .await;
    assert_eq!(impostor.did, a.did, "the same master yields the same service address");
    impostor.introduce(&b).await;
    let imp_conv = open_conversation(&impostor.node, &impostor.did, &b.did).await;
    let refused =
        send_message(&impostor.node, &impostor.did, &imp_conv, "from a second node").await;
    let failed =
        wait_for_state(&impostor.node, &impostor.did, &refused, "failed", FAIL_BUDGET).await;
    let row = message_row(&impostor.node, &impostor.did, &imp_conv, &refused).await;
    assert!(failed, "a re-presented key must be refused, not re-pinned; row: {row:?}");

    deliver(&a, &b, &conv, "the real A still works").await;
    assert_eq!(
        bodies(&b.history_with(&a).await),
        ["from the real A", "the real A still works"],
        "the second node's message never reached B's history"
    );
    for p in [impostor, b, a] {
        p.teardown().await;
    }
}

/// The proxy's capability gate: a guest may not call the `conversation`
/// interface of *another* service, and the refusal names that policy.
#[tokio::test]
async fn a_guest_calling_conversation_on_another_service_is_denied_by_the_proxy() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    let wasm = wasm();
    let a = Peer::boot(None, default_role(), &wasm).await;
    let b = Peer::boot(Some(a.registry()), default_role(), &wasm).await;

    let reply = fixture_run(
        &a.node,
        &a.did,
        &json!({
            "op": "proxy-call-cross-service-native",
            "target": b.did,
            "interface": "conversation",
            "method": "deliver",
            "params": "{}"
        }),
    )
    .await;
    let error = reply["ok"]["error"].as_str().unwrap_or_else(|| panic!("no refusal: {reply}"));
    assert!(error.contains("PermissionDenied"), "{error}");
    assert!(error.contains("native capability"), "the gate, not the callee, refused: {error}");
    let listed = fixture_run(&b.node, &b.did, &json!({"op": "list-conversations"})).await;
    assert!(
        listed["ok"]["conversations"].as_array().unwrap().is_empty(),
        "nothing reached B: {listed}"
    );
    b.teardown().await;
    a.teardown().await;
}

/// The same-service exemption lets a guest reach its *own* `conversation`
/// `deliver` arm. This proves the proxy gate lets the call through and the
/// arm itself refuses an envelope it cannot open. The narrower rule, that a
/// service cannot deliver a message to itself, needs a session that opens,
/// which no guest can build; the unit test
/// `self_injection_via_same_service_is_refused` covers that guard.
#[tokio::test]
async fn a_guest_reaching_its_own_deliver_arm_is_still_refused_by_the_arm() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    let wasm = wasm();
    let a = Peer::boot(None, default_role(), &wasm).await;

    // A decodable envelope that no session backs: the arm decodes it, then
    // refuses it. `message_body` is a well-formed but meaningless Olm
    // message: version 3, a ratchet key, chain index 0, one ciphertext byte
    // and a MAC.
    let sender_key = vec![7u8; 32];
    let message_body = "AwogAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEQACIBAAAAAAAAAAAA";
    let envelope = json!({
        "peer_address": a.did,
        "sender_address": "did:key:zSomeoneElse",
        "sender_sig_key": sender_key,
        "message": { "type": 1, "body": message_body },
    });
    let reply = fixture_run(
        &a.node,
        &a.did,
        &json!({
            "op": "proxy-call-self",
            "service_id": a.did,
            "interface": "conversation",
            "method": "deliver",
            "params": envelope.to_string(),
        }),
    )
    .await;
    let error = reply["err"].as_str().unwrap_or_else(|| panic!("no refusal: {reply}"));
    assert!(!error.contains("native capability"), "the proxy gate must let it through: {error}");
    assert!(error.to_lowercase().contains("permission denied"), "the arm refuses it: {error}");
    a.teardown().await;
}

/// Prekey requests are limited per requesting peer. A stranger past the
/// limit is refused, and that does not drain the pool an honest peer draws
/// from: A still establishes a session with B afterwards.
#[tokio::test]
async fn prekey_requests_past_the_hourly_limit_are_refused_and_do_not_starve_an_honest_peer() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    let wasm = wasm();
    let a = Peer::boot(None, default_role(), &wasm).await;
    let role =
        AppSandboxRole { conversation_prekey_requests_per_peer_per_hour: 2, ..default_role() };
    let b = Peer::boot(Some(a.registry()), role, &wasm).await;

    let mut stranger = SyneroymClient::new_with_identity(
        b.did.clone(),
        a.registry().to_string(),
        Identity::generate().unwrap(),
    )
    .with_registry_dht(false);
    stranger.connect().await.expect("stranger connect failed");
    for n in 1..=2 {
        let bundle = stranger.request("conversation", "prekey-bundle", json!({})).await;
        assert!(bundle.is_ok(), "request {n} is inside the limit: {bundle:?}");
    }
    let third = stranger.request("conversation", "prekey-bundle", json!({})).await;
    let refusal = format!("{third:?}");
    assert!(
        refusal.to_lowercase().contains("permission denied"),
        "the third request in an hour is refused by the limit, not by a dropped link: {refusal}"
    );
    stranger.shutdown().await.ok();

    let conv = open_conversation(&a.node, &a.did, &b.did).await;
    deliver(&a, &b, &conv, "the pool still has keys").await;
    b.teardown().await;
    a.teardown().await;
}

/// The pending-message ceiling is per conversation: filling one leaves
/// another conversation on the same node free to send.
#[tokio::test]
async fn the_pending_quota_of_one_conversation_does_not_block_another() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    let wasm = wasm();
    let role = AppSandboxRole { conversation_max_pending_per_conversation: 2, ..default_role() };
    let a = Peer::boot(None, role, &wasm).await;
    let dead = |_| substrate::derive_did_key(&Identity::generate().unwrap().public_key());
    let (conv1, conv2) = (
        open_conversation(&a.node, &a.did, &dead(1)).await,
        open_conversation(&a.node, &a.did, &dead(2)).await,
    );

    send_message(&a.node, &a.did, &conv1, "one").await;
    send_message(&a.node, &a.did, &conv1, "two").await;
    let third = try_send_message(&a.node, &a.did, &conv1, "three").await;
    assert!(
        third["err"].as_str().is_some_and(|e| e.contains("QuotaExceeded")),
        "the third pending message is over the ceiling: {third}"
    );
    send_message(&a.node, &a.did, &conv2, "other conversation, still free").await;
    a.teardown().await;
}

/// A sender timestamp a year in the future is refused by the clock-skew
/// check; one a year in the past is accepted and kept as claimed. Both
/// messages are signed by the real key, so only the skew check differs.
#[tokio::test]
async fn a_future_sender_timestamp_is_refused_and_a_past_one_is_kept() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    let wasm = wasm();
    let a = Peer::boot(None, default_role(), &wasm).await;
    let b = Peer::boot(Some(a.registry()), default_role(), &wasm).await;
    let conv = open_conversation(&a.node, &a.did, &b.did).await;
    deliver(&a, &b, &conv, "establishes the session").await;
    let now_ms = now_ms();

    let past = now_ms - YEAR_MS;
    override_next_send(
        &a.did,
        SendOverride { sender_timestamp_ms: Some(past), ..SendOverride::default() },
    );
    let old = deliver(&a, &b, &conv, "a year old").await;
    let held = message_row(&b.node, &b.did, &b.conversation_with(&a), &old).await.unwrap();
    assert_eq!(held["sender-timestamp"], past, "kept as the sender claimed: {held}");

    override_next_send(
        &a.did,
        SendOverride { sender_timestamp_ms: Some(now_ms + YEAR_MS), ..SendOverride::default() },
    );
    let future = send_message(&a.node, &a.did, &conv, "a year ahead").await;
    assert!(
        wait_for_state(&a.node, &a.did, &future, "failed", FAIL_BUDGET).await,
        "a far-future timestamp is refused on arrival"
    );
    assert!(
        message_row(&b.node, &b.did, &b.conversation_with(&a), &future).await.is_none(),
        "and never stored"
    );
    b.teardown().await;
    a.teardown().await;
}

fn now_ms() -> i64 {
    let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    i64::try_from(since_epoch.as_millis()).unwrap()
}

/// A service with no installed instance certificate cannot present an
/// identity to a peer, so its send fails at once -- terminally, and naming
/// the certificate.
#[tokio::test]
async fn a_send_with_no_instance_certificate_fails_naming_the_certificate() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    let wasm = wasm();
    let a = Peer::boot_as(
        Identity::generate().unwrap(),
        None,
        default_role(),
        &wasm,
        Deploy::NoCertificate,
    )
    .await;
    let peer = substrate::derive_did_key(&Identity::generate().unwrap().public_key());
    let conv = open_conversation(&a.node, &a.did, &peer).await;

    let id = send_message(&a.node, &a.did, &conv, "no certificate").await;
    assert!(wait_for_state(&a.node, &a.did, &id, "failed", FAIL_BUDGET).await);
    let row = message_row(&a.node, &a.did, &conv, &id).await.unwrap();
    assert!(
        row["last-error"].as_str().is_some_and(|e| e.contains("instance certificate")),
        "the failure names the certificate: {row}"
    );
    a.teardown().await;
}
