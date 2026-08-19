#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The guest-facing durable outbox, end to end across two real substrates
//! ([ADR-0023](../../../docs/decisions/0023-durable-async-primitives.md)
//! §2, §4, §5).
//!
//! Every *property* these two cases touch already has an in-process test in
//! `syneroym-router`. What only an e2e can prove is the **sequence**: a
//! guest hands a call to its outbox, the process that owns that outbox is
//! torn down and restarted, and the call is still there afterwards and
//! still delivers. A restart with a live queue is not expressible in a
//! unit test, which is the whole reason these two exist.
//!
//! Each stage is asserted through the `proxy-outbox` operator verb --
//! queued, still queued as the *same* item after the restart, gone once
//! delivered -- rather than inferred from a side effect. That is the
//! correction B1's own review had to make to its equivalent test, and
//! copying its earlier shape would have reproduced the same weakness.
//!
//! Skips when the `proxy-test`/`greeter` wasm artifacts are absent
//! (`cargo build --target wasm32-wasip2 --release` in each), matching every
//! other guest-driven test in this tree.

use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use ed25519_dalek::VerifyingKey;
use reqwest::Client;
use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_core::{
    config::{
        AppSandboxRole, ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole,
        IrohParentConfig, LogTarget, ServiceRegistryRole, SubstrateConfig,
    },
    dht_registry::{EndpointInfo, EndpointMechanism, EndpointType, RegistryClient},
    test_constants,
};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_sdk::SyneroymClient;
use syneroym_substrate::identity;
use tokio::{
    sync::{Mutex, mpsc, mpsc::Sender},
    task::JoinHandle,
    time,
};

#[path = "common/retry.rs"]
mod retry;

/// The highest block claimed in this directory before this file is
/// `durable_outbox_e2e.rs`'s 13_200-13_402. Every node takes its own
/// **hundred**-port block, not a ten-port one: the Iroh relay binds more
/// than the single HTTP port it is given, so nodes spaced ten apart
/// collide with "address already in use" on the second boot. The two tests
/// here also never share a block, since `cargo test` runs them
/// concurrently in one binary.
const DELIVERS_PORTS: (u16, u16, u16, u16, u16, u16) =
    (13_500, 13_501, 13_502, 13_600, 13_601, 13_602);
const DLQ_PORTS: (u16, u16, u16, u16, u16, u16) = (13_700, 13_701, 13_702, 13_800, 13_801, 13_802);

/// Long enough that the item is still waiting when node B finishes booting
/// again -- a budget that ran out first would dead-letter the call for a
/// reason the test is not about.
const DELIVERY_ATTEMPT_BUDGET: u8 = 200;
/// Short enough to exhaust promptly, since exhausting is the point.
const DLQ_ATTEMPT_BUDGET: u8 = 3;
/// Node B comes back on a *different* address on purpose, so the stale
/// record points at nothing until the redeploy is done and the fresh
/// record is published. Not required for correctness -- a "service not
/// found" answer is retried, so a reboot on the same address converges
/// too -- but it keeps this test's timing about the outbox rather than
/// about how quickly the target's services happen to come up.
const NODE_B_REBOOT_PORTS: (u16, u16, u16) = (13_900, 13_901, 13_902);

/// Not sharing a port block keeps the two tests here from colliding, but
/// they still each boot two full substrate instances (real iroh QUIC
/// socket, self-hosted relay, mainline DHT, wasmtime), and running that
/// many concurrently starves the CI runner's CPU badly enough that iroh's
/// QUIC path validation times out with "no viable network path exists"
/// even though nothing is actually broken. Serializing full-substrate-
/// instance lifetimes within this binary (same fix as `tests/common/mod.rs`'s
/// `SUBSTRATE_TEST_LOCK`) avoids it.
static SUBSTRATE_TEST_LOCK: Mutex<()> = Mutex::const_new(());

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
fn fast_queue_role(max_attempts: u8) -> AppSandboxRole {
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

/// A full substrate booted from a caller-owned directory, so the same
/// identity can be torn down and rebooted later in the same test. Copied
/// in shape from `durable_outbox_e2e.rs`'s own `Node`.
struct Node {
    substrate_client: SyneroymClient,
    registry_url: String,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
}

impl Node {
    async fn boot(
        base_path: PathBuf,
        iroh_port: u16,
        registry_port: u16,
        gateway_port: u16,
        shared_registry_url: Option<String>,
        owner: &Identity,
        app_sandbox: Option<AppSandboxRole>,
    ) -> Self {
        let mut config = SubstrateConfig {
            app_local_data_dir: base_path.join("data"),
            app_data_dir: base_path.join("user_data"),
            app_cache_dir: base_path.join("cache"),
            app_log_dir: base_path.join("logs"),
            profile: "full".to_string(),
            ..SubstrateConfig::default()
        };
        config.resolve_paths();
        config.logging.target = LogTarget::Stdout;
        config.roles.coordinator = Some(CoordinatorRole {
            iroh: Some(CoordinatorIrohConfig {
                enable_relay: true,
                http_bind_address: format!("0.0.0.0:{iroh_port}"),
                ..Default::default()
            }),
            ..Default::default()
        });
        config.roles.community_registry = Some(ServiceRegistryRole {
            http_bind_address: format!("0.0.0.0:{registry_port}"),
            ..Default::default()
        });
        let own_registry_url = format!("http://localhost:{registry_port}");
        let effective_registry_url = shared_registry_url.unwrap_or(own_registry_url);
        config.substrate.registry_url = Some(effective_registry_url.clone());
        config.substrate.enable_bep0044_dht = false;
        config.parent_coordinator.iroh =
            Some(IrohParentConfig { url: format!("http://localhost:{iroh_port}") });
        config.roles.client_gateway =
            Some(ClientGatewayRole { http_port: gateway_port, ..Default::default() });
        config.iam.admin_ucan_root = Some(substrate::derive_did_key(&owner.public_key()));
        if let Some(role) = app_sandbox {
            config.roles.app_sandbox = Some(role);
            // One connect attempt per delivery. The proxy's own retry loop
            // sits *underneath* the outbox here, so leaving it at three
            // multiplies every queued attempt by three call timeouts for
            // nothing -- the outbox is already the retry mechanism.
            config.retry.max_attempts = 1;
        }

        let state = identity::setup_substrate_identity(&config.identity, &config.app_data_dir)
            .expect("failed to setup identity");
        let substrate_service_id = state.did.clone();

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        let runtime =
            syneroym_substrate::init(config.clone()).await.expect("failed to initialize runtime");
        let config_clone = config.clone();
        let substrate_handle = tokio::spawn(async move {
            syneroym_substrate::run_with_signal(config_clone, runtime, async {
                let _ = shutdown_rx.recv().await;
            })
            .await
            .expect("substrate failed to run");
        });

        let mut substrate_client = SyneroymClient::new_with_identity(
            substrate_service_id,
            effective_registry_url.clone(),
            Identity::from_bytes(&owner.to_bytes()),
        )
        .with_registry_dht(false);
        substrate_client
            .wait_for_ready(Duration::from_secs(30))
            .await
            .expect("substrate did not become available in time");
        substrate_client.inject_kek(hex::encode([0xcdu8; 32])).await.expect("inject_kek failed");

        Self {
            substrate_client,
            registry_url: effective_registry_url,
            shutdown_tx,
            substrate_handle,
        }
    }

    fn did(&self) -> &str {
        self.substrate_client.service_id()
    }

    async fn teardown(mut self) {
        let _ = self.substrate_client.shutdown().await;
        let _ = self.shutdown_tx.send(()).await;
        let _ = self.substrate_handle.await;
    }
}

/// Publishes `service_id`'s endpoint record so another node's proxy can
/// resolve it. Without this the proxy answers `service-not-found`, which is
/// *terminal* -- so an unpublished target never reaches the outbox at all,
/// and the "unreachable" case these tests need would not be the one they
/// mean to exercise.
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
async fn deploy_guest(node: &mut Node, master: &Identity, wasm: Vec<u8>) -> String {
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
    RegistryClient::new(false, Some(node.registry_url.clone()))
        .publish_master_anchor(&service_id, vec![], None, master, true)
        .await
        .expect("failed to publish the caller master's anchor");

    // Published as well as deployed: the test drives the guest through an
    // ordinary client, which resolves it through the registry like any
    // other caller would.
    let mechanisms =
        node.substrate_client.lookup().await.expect("node lookup failed").info.mechanisms;
    publish_endpoint(&service_id, node.did(), mechanisms, master, &node.registry_url).await;
    service_id
}

/// Drives the guest's own `enqueue-peer` export -- real guest code calling
/// `syneroym:proxy/proxy::enqueue`, not a Rust-level fake.
async fn guest_enqueue(
    node: &Node,
    guest_service_id: &str,
    target_did: &str,
    idempotency_key: &str,
) -> Result<Value, String> {
    let mut client = SyneroymClient::new_with_identity(
        guest_service_id.to_string(),
        node.registry_url.clone(),
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

async fn outbox_keys(node: &Node, service_id: &str) -> Vec<String> {
    node.substrate_client
        .proxy_outbox(service_id.to_string())
        .await
        .expect("proxy-outbox failed")
        .into_iter()
        .map(|i| i.idempotency_key)
        .collect()
}

async fn dead_letter_keys(node: &Node, service_id: &str) -> Vec<String> {
    node.substrate_client
        .proxy_dead_letters(service_id.to_string())
        .await
        .expect("proxy-dead-letters failed")
        .into_iter()
        .map(|i| i.idempotency_key)
        .collect()
}

/// Polls `check` until it holds or `budget` runs out, returning whether it
/// held. Polling rather than sleeping a fixed span: the queue tick is 1s,
/// so a fixed wait would either be flaky or needlessly slow.
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
        time::sleep(Duration::from_millis(500)).await;
    }
    false
}

fn artifacts() -> Option<(Vec<u8>, Vec<u8>)> {
    let proxy = fs::read(test_constants::proxy_test_wasm_path()).ok()?;
    let greeter = fs::read(test_constants::greeter_wasm_path()).ok()?;
    Some((proxy, greeter))
}

/// **Why a target that is merely restarting does not dead-letter here.**
/// When the target node comes back it republishes its own endpoint record
/// before its services finish coming up, so a delivery attempt lands in a
/// window where the address is right and the service is not there yet. A
/// "service not found" answer is therefore treated as retryable rather
/// than terminal -- bounded by the ordinary attempt budget, so a target
/// that is genuinely gone still dead-letters, just not on the first hit.
/// Without that, the item would be given up on during exactly the outage
/// this queue exists to survive.
///
/// The sequence no in-process test can cover: a guest queues a call to a
/// node that is down, the **calling** substrate restarts, the node comes
/// back, and the call lands -- with the outbox itself asserted at every
/// stage rather than inferred.
#[tokio::test]
async fn a_queued_guest_call_to_an_offline_node_lands_after_it_returns() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let _ = ring::default_provider().install_default();
    let (proxy_wasm, greeter_wasm) =
        artifacts().expect("proxy-test/greeter wasm artifacts not built");
    let (a_iroh, a_reg, a_gw, b_iroh, b_reg, b_gw) = DELIVERS_PORTS;

    let caller_dir = tempfile::tempdir().unwrap();
    let target_dir = tempfile::tempdir().unwrap();
    let owner = Identity::generate().unwrap();

    // Node A hosts the registry, so its own restart does not take the
    // record store down with it -- the data directory is caller-owned and
    // survives the reboot.
    let mut node_a = Node::boot(
        caller_dir.path().to_path_buf(),
        a_iroh,
        a_reg,
        a_gw,
        None,
        &owner,
        Some(fast_queue_role(DELIVERY_ATTEMPT_BUDGET)),
    )
    .await;
    let shared_registry = node_a.registry_url.clone();

    let node_b = Node::boot(
        target_dir.path().to_path_buf(),
        b_iroh,
        b_reg,
        b_gw,
        Some(shared_registry.clone()),
        &owner,
        None,
    )
    .await;

    // The target: an ordinary greeter on node B, published so node A's
    // proxy can resolve it.
    let target_master = Identity::generate().unwrap();
    let target_did = substrate::derive_did_key(&target_master.public_key());
    node_b
        .substrate_client
        .deploy_svc_wasm(
            target_did.clone(),
            vec![test_constants::GREETER_INTERFACE_NAME.to_string()],
            greeter_wasm.clone(),
            syneroym_sdk::Publication::Private,
            None,
        )
        .await
        .expect("target deploy failed");
    let node_b_info =
        node_b.substrate_client.lookup().await.expect("node B lookup failed").info.mechanisms;
    let node_b_did = node_b.did().to_string();
    publish_endpoint(
        &target_did,
        &node_b_did,
        node_b_info.clone(),
        &target_master,
        &shared_registry,
    )
    .await;

    // The caller: the guest fixture on node A, certified so `enqueue` is
    // not refused up front.
    let caller_master = Identity::generate().unwrap();
    let guest_did = deploy_guest(&mut node_a, &caller_master, proxy_wasm.clone()).await;

    // Node B goes down. The record stays in the registry, so the proxy
    // resolves an address and then fails to connect -- a *transport*
    // failure, which is the retryable case the outbox exists for.
    node_b.teardown().await;
    time::sleep(Duration::from_secs(1)).await;
    // Re-published while it is down, on purpose. A target whose record has
    // gone resolves to nothing, and "no such service" is *terminal* -- so
    // without this the enqueue would be refused outright rather than
    // queued, and the test would be exercising a different case than the
    // one it means to.
    publish_endpoint(
        &target_did,
        &node_b_did,
        node_b_info.clone(),
        &target_master,
        &shared_registry,
    )
    .await;

    // The guest enqueues. Fire-and-forget: it gets no delivery outcome.
    guest_enqueue(&node_a, &guest_did, &target_did, "msg-1")
        .await
        .expect("enqueue must be accepted for delivery");

    assert!(
        wait_until(Duration::from_secs(30), || async {
            outbox_keys(&node_a, &guest_did).await == vec!["msg-1".to_string()]
        })
        .await,
        "the queued call must be visible in the outbox, not merely inferred"
    );

    // A second enqueue of the same logical operation is a no-op at the
    // sender: the queue key *is* the idempotency key.
    guest_enqueue(&node_a, &guest_did, &target_did, "msg-1").await.ok();
    assert_eq!(
        outbox_keys(&node_a, &guest_did).await,
        vec!["msg-1".to_string()],
        "re-enqueueing one logical operation must not produce a second queued item"
    );

    // Restart the calling substrate -- the step no in-process test covers.
    node_a.teardown().await;
    node_a = Node::boot(
        caller_dir.path().to_path_buf(),
        a_iroh,
        a_reg,
        a_gw,
        None,
        &owner,
        Some(fast_queue_role(DELIVERY_ATTEMPT_BUDGET)),
    )
    .await;
    assert_eq!(
        outbox_keys(&node_a, &guest_did).await,
        vec!["msg-1".to_string()],
        "the same item must still be queued across the restart, not a different or duplicated one"
    );

    // The community registry keeps its records **in memory**, so node A's
    // own restart empties the registry it hosts. Re-published here because
    // otherwise the worker's next attempt resolves nothing, and a target
    // that resolves to nothing is deliberately terminal -- the item would
    // dead-letter for a reason that is an artifact of this harness rather
    // than of the behavior under test. In a real deployment the record
    // outlives a brief outage: it carries its own TTL and its owner
    // republishes it.
    publish_endpoint(
        &target_did,
        &node_b_did,
        node_b_info.clone(),
        &target_master,
        &shared_registry,
    )
    .await;
    // The master anchor went with it, and the receiving node needs that
    // anchor to verify the guest's instance certificate on every delivery
    // attempt. Without it the handshake is rejected, which is a transport
    // failure -- so the item neither lands nor dead-letters, it just
    // retries forever.
    RegistryClient::new(false, Some(shared_registry.clone()))
        .publish_master_anchor(&guest_did, vec![], None, &caller_master, true)
        .await
        .expect("failed to republish the caller master's anchor after the restart");
    assert!(
        dead_letter_keys(&node_a, &guest_did).await.is_empty(),
        "a restart must not turn a waiting item into a dead letter"
    );

    // Node B returns: same identity and same directory, different address.
    let (b_iroh2, b_reg2, b_gw2) = NODE_B_REBOOT_PORTS;
    let node_b = Node::boot(
        target_dir.path().to_path_buf(),
        b_iroh2,
        b_reg2,
        b_gw2,
        Some(shared_registry.clone()),
        &owner,
        None,
    )
    .await;
    // Re-deployed as well as rebooted: a substrate does not bring its
    // deployed services back up by itself, and this test is about node
    // A's outbox surviving, not about node B's deployment persistence.
    node_b
        .substrate_client
        .deploy_svc_wasm(
            target_did.clone(),
            vec![test_constants::GREETER_INTERFACE_NAME.to_string()],
            greeter_wasm,
            syneroym_sdk::Publication::Private,
            None,
        )
        .await
        .expect("target redeploy failed");
    let listed = node_b.substrate_client.list_svcs().await.expect("list on node B failed");
    assert!(
        listed.iter().any(|svc| svc.service_id == target_did
            && svc.interfaces.contains(&test_constants::GREETER_INTERFACE_NAME.to_string())),
        "node B must know the target again before its record is republished, got: {:?}",
        listed.iter().map(|s| (&s.service_id, &s.interfaces)).collect::<Vec<_>>()
    );

    // Its *fresh* mechanisms, not the ones captured before the outage: a
    // reboot keeps the node's identity (same directory) but not
    // necessarily its Iroh address, and republishing the stale one leaves
    // the worker retrying against somewhere nothing is listening.
    let node_b_info_after = node_b
        .substrate_client
        .lookup()
        .await
        .expect("node B lookup after reboot failed")
        .info
        .mechanisms;
    publish_endpoint(
        &target_did,
        node_b.did(),
        node_b_info_after,
        &target_master,
        &shared_registry,
    )
    .await;

    // `node_a`'s own connection sat idle through node B's full reboot,
    // redeploy, and lookup just above, long enough under CI's scheduling
    // pressure for the peer to abandon that idle path ("no viable network
    // path exists: last path abandoned by peer"; same root cause fixed
    // throughout this crate's e2e tests). `outbox_keys` panics on any
    // error, so a stale connection here would abort the test with a
    // misleading "proxy-outbox failed" instead of the poll loop below ever
    // getting a chance to run -- redial once, before entering that loop,
    // rather than letting its first iteration find out the hard way.
    let _ = crate::call_with_reconnect!(
        node_a.substrate_client,
        node_a.substrate_client.proxy_outbox(guest_did.clone()).await
    );
    assert!(
        wait_until(Duration::from_secs(120), || async {
            outbox_keys(&node_a, &guest_did).await.is_empty()
        })
        .await,
        "the queued call must be delivered once its target returns; outbox still holds {:?}",
        outbox_keys(&node_a, &guest_did).await
    );
    assert!(
        dead_letter_keys(&node_a, &guest_did).await.is_empty(),
        "a delivered call must leave through delivery, not through the dead-letter table"
    );

    // It stays delivered: nothing re-queues it on a later tick.
    time::sleep(Duration::from_secs(3)).await;
    assert!(
        outbox_keys(&node_a, &guest_did).await.is_empty(),
        "a delivered item must not reappear in the outbox"
    );

    node_b.teardown().await;
    node_a.teardown().await;
}

/// The terminal half: a target that never comes back exhausts the attempt
/// budget, lands in the dead-letter table where an operator can see it, and
/// is replayable from there.
#[tokio::test]
async fn a_permanently_unreachable_target_lands_in_the_dlq_and_replays() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let _ = ring::default_provider().install_default();
    let (proxy_wasm, greeter_wasm) =
        artifacts().expect("proxy-test/greeter wasm artifacts not built");
    let (a_iroh, a_reg, a_gw, b_iroh, b_reg, b_gw) = DLQ_PORTS;

    let caller_dir = tempfile::tempdir().unwrap();
    let target_dir = tempfile::tempdir().unwrap();
    let owner = Identity::generate().unwrap();

    let mut node_a = Node::boot(
        caller_dir.path().to_path_buf(),
        a_iroh,
        a_reg,
        a_gw,
        None,
        &owner,
        Some(fast_queue_role(DLQ_ATTEMPT_BUDGET)),
    )
    .await;
    let shared_registry = node_a.registry_url.clone();

    let node_b = Node::boot(
        target_dir.path().to_path_buf(),
        b_iroh,
        b_reg,
        b_gw,
        Some(shared_registry.clone()),
        &owner,
        None,
    )
    .await;
    let target_master = Identity::generate().unwrap();
    let target_did = substrate::derive_did_key(&target_master.public_key());
    node_b
        .substrate_client
        .deploy_svc_wasm(
            target_did.clone(),
            vec![test_constants::GREETER_INTERFACE_NAME.to_string()],
            greeter_wasm,
            syneroym_sdk::Publication::Private,
            None,
        )
        .await
        .expect("target deploy failed");
    let node_b_info =
        node_b.substrate_client.lookup().await.expect("node B lookup failed").info.mechanisms;
    let node_b_did = node_b.did().to_string();
    publish_endpoint(
        &target_did,
        &node_b_did,
        node_b_info.clone(),
        &target_master,
        &shared_registry,
    )
    .await;

    let caller_master = Identity::generate().unwrap();
    let guest_did = deploy_guest(&mut node_a, &caller_master, proxy_wasm).await;

    // Down for good this time. The record stays published, so every
    // attempt resolves an address and then fails to connect.
    node_b.teardown().await;
    time::sleep(Duration::from_secs(1)).await;
    // Re-published while down, for the same reason as the delivery case: a
    // target that resolves to nothing is terminal at the call, which is a
    // different failure than the permanently-unreachable one under test.
    publish_endpoint(&target_did, &node_b_did, node_b_info, &target_master, &shared_registry).await;

    guest_enqueue(&node_a, &guest_did, &target_did, "doomed-1")
        .await
        .expect("enqueue must be accepted for delivery");

    assert!(
        wait_until(Duration::from_secs(300), || async {
            dead_letter_keys(&node_a, &guest_did).await == vec!["doomed-1".to_string()]
        })
        .await,
        "the item must reach the dead-letter table once its budget is exhausted; outbox {:?}, \
         dead letters {:?}",
        outbox_keys(&node_a, &guest_did).await,
        dead_letter_keys(&node_a, &guest_did).await
    );
    assert!(
        outbox_keys(&node_a, &guest_did).await.is_empty(),
        "a dead-lettered item must have left the outbox"
    );

    // Replay puts it back on the outbox for another attempt -- it never
    // executes inline, so the dead letter is consumed and a queued item
    // appears in its place.
    let dead = node_a
        .substrate_client
        .proxy_dead_letters(guest_did.clone())
        .await
        .expect("proxy-dead-letters failed");
    assert_eq!(dead.len(), 1);
    node_a
        .substrate_client
        .proxy_replay(guest_did.clone(), dead[0].id)
        .await
        .expect("proxy-replay failed");

    assert!(
        dead_letter_keys(&node_a, &guest_did).await.is_empty(),
        "replay must consume the dead letter"
    );
    assert!(
        wait_until(Duration::from_secs(10), || async {
            !outbox_keys(&node_a, &guest_did).await.is_empty()
                || !dead_letter_keys(&node_a, &guest_did).await.is_empty()
        })
        .await,
        "replay must re-enqueue the call rather than discard it"
    );

    node_a.teardown().await;
}
