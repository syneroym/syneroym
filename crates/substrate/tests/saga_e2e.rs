#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Saga compensations, end to end across two real substrates (ADR-0023
//! §7, as amended).
//!
//! Every *property* the walk touches already has an in-process test in
//! `syneroym-router`. What only an e2e can prove is the **sequence**: a
//! guest drives a real cross-node workflow through its own WIT exports, the
//! walk undoes it in the right order on a real second node, and -- the
//! step no in-process test can express -- a saga survives a real process
//! restart and is still compensated by its own deadline once the vault is
//! unlocked again.
//!
//! Fails, rather than skips, when the `saga-test` wasm artifact is absent
//! (`cargo component build --release --target wasm32-wasip2` in
//! `test-components/saga-test`) -- a passing run against a missing
//! component is exactly the false confidence this test exists to prevent.

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
use syneroym_rpc::SagaState;
use syneroym_sdk::SyneroymClient;
use syneroym_substrate::identity;
use tokio::{
    sync::{Mutex, mpsc, mpsc::Sender},
    task::JoinHandle,
    time,
};

#[path = "common/retry.rs"]
mod retry;

/// This file's own hundred-port blocks, distinct from every other file in
/// this directory (the highest claimed elsewhere at the time this was
/// written is `proxy_outbox_e2e.rs`'s 13_900-13_902).
const REVERSE_WALK_PORTS: (u16, u16, u16, u16, u16, u16) =
    (14_000, 14_001, 14_002, 14_100, 14_101, 14_102);
const DEADLINE_PORTS: (u16, u16, u16, u16, u16, u16) =
    (14_200, 14_201, 14_202, 14_300, 14_301, 14_302);

/// Each test in this binary boots two full substrate instances (real iroh
/// QUIC socket, self-hosted relay, mainline DHT, wasmtime). The default
/// test harness runs `#[tokio::test]` functions in one binary concurrently,
/// and enough full substrates competing for the CI runner's CPU at once
/// starves it badly enough that iroh's QUIC path validation times out with
/// "no viable network path exists" even though nothing is actually broken.
/// Serializing full-substrate-instance lifetimes within this binary (same
/// fix as `tests/common/mod.rs`'s `SUBSTRATE_TEST_LOCK`) avoids it.
static SUBSTRATE_TEST_LOCK: Mutex<()> = Mutex::const_new(());

/// A saga must not have to wait out the production retry window for a test
/// to see its undos land, and the sweep tick must be fast enough that a
/// 60s-deadline saga (test 2) is picked up promptly once past it.
fn fast_saga_role() -> AppSandboxRole {
    AppSandboxRole {
        queue_tick_secs: 1,
        saga_max_deadline_secs: 86_400,
        ..AppSandboxRole::default()
    }
}

/// A full substrate booted from a caller-owned directory, so the same
/// identity can be torn down and rebooted later in the same test. Copied in
/// shape from `proxy_outbox_e2e.rs`'s own `Node`, split into
/// `boot_locked`/`unlock` (this file's own addition): a restart must be
/// observable *before* the KEK is injected, which `boot`'s own
/// unconditional injection never let a test see.
struct Node {
    substrate_client: SyneroymClient,
    registry_url: String,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
}

impl Node {
    async fn boot_locked(
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
        config.parent_coordinator.iroh =
            Some(IrohParentConfig { url: format!("http://localhost:{iroh_port}") });
        config.roles.client_gateway =
            Some(ClientGatewayRole { http_port: gateway_port, ..Default::default() });
        config.iam.admin_ucan_root = Some(substrate::derive_did_key(&owner.public_key()));
        if let Some(role) = app_sandbox {
            config.roles.app_sandbox = Some(role);
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
        );
        substrate_client
            .wait_for_ready(Duration::from_secs(30))
            .await
            .expect("substrate did not become available in time");

        Self {
            substrate_client,
            registry_url: effective_registry_url,
            shutdown_tx,
            substrate_handle,
        }
    }

    async fn unlock(&mut self) {
        // Some callers reach this right after `boot_locked` (no idle gap,
        // never fails), others reach it after the connection sat idle
        // through a teardown/reboot cycle plus a real wall-clock sleep
        // ("no viable network path exists: last path abandoned by peer";
        // same root cause fixed throughout this crate's e2e tests, e.g.
        // `binding_push_e2e.rs`). `SyneroymClient::connect` no-ops on an
        // already-`Some` connection, so recovering means an explicit
        // `shutdown`-then-`connect` (redial) before one retry, not just
        // retrying the same request on the same dead connection.
        crate::call_with_reconnect!(
            self.substrate_client,
            self.substrate_client.inject_kek(hex::encode([0xcdu8; 32])).await
        );
    }

    async fn boot(
        base_path: PathBuf,
        iroh_port: u16,
        registry_port: u16,
        gateway_port: u16,
        shared_registry_url: Option<String>,
        owner: &Identity,
        app_sandbox: Option<AppSandboxRole>,
    ) -> Self {
        let mut node = Self::boot_locked(
            base_path,
            iroh_port,
            registry_port,
            gateway_port,
            shared_registry_url,
            owner,
            app_sandbox,
        )
        .await;
        node.unlock().await;
        node
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
/// resolve it. Copied from `proxy_outbox_e2e.rs`.
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

/// Deploys the `saga-test` guest as `master`'s own DID. `with_cert` mints
/// and installs an instance certificate -- required for the driver, since
/// `begin` refuses a caller with no unexpired one; the participant needs
/// none, as it never originates a saga call itself.
async fn deploy_guest(
    node: &mut Node,
    master: &Identity,
    wasm: Vec<u8>,
    with_cert: bool,
) -> String {
    let service_id = substrate::derive_did_key(&master.public_key());
    let instance_certificate = if with_cert {
        // `node`'s connection was dialed and proven live by its own
        // `wait_for_ready` during `Node::boot`, then may have sat idle
        // through a second full substrate boot (and that node's own
        // `deploy_guest`, when this is the driver) before this, the
        // first real call on it, reuses it -- long enough under CI's
        // scheduling pressure for the peer to abandon that idle path
        // ("no viable network path exists: last path abandoned by
        // peer"; same root cause as `app_instance_identity_e2e.rs`'s
        // and `supervisor_alerts_e2e.rs`'s own fixes).
        // `SyneroymClient::connect` no-ops on an already-`Some`
        // connection, so recovering means an explicit
        // `shutdown`-then-`connect` (redial) before one retry, not just
        // retrying the same request on the same dead connection.
        let identity = match node.substrate_client.instance_identity(&service_id).await {
            Ok(identity) => identity,
            Err(_) => {
                node.substrate_client
                    .shutdown()
                    .await
                    .expect("failed to reset node's stale connection");
                node.substrate_client.connect().await.expect("failed to reconnect node");
                node.substrate_client
                    .instance_identity(&service_id)
                    .await
                    .expect("instance-identity query failed")
            }
        };
        let pubkey_bytes: [u8; 32] = hex::decode(&identity.pubkey_hex)
            .expect("instance pubkey is not hex")
            .try_into()
            .expect("instance pubkey is not 32 bytes");
        let instance_pubkey = VerifyingKey::from_bytes(&pubkey_bytes).unwrap();
        Some(
            DelegationCertificate::issue(
                master,
                instance_pubkey,
                3600,
                SCOPE_SERVICE_INSTANCE.to_string(),
            )
            .unwrap(),
        )
    } else {
        None
    };

    node.substrate_client
        .deploy_svc_wasm(
            service_id.clone(),
            vec![
                test_constants::SAGA_TEST_DRIVER_INTERFACE.to_string(),
                test_constants::SAGA_TEST_PARTICIPANT_INTERFACE.to_string(),
            ],
            wasm,
            None,
            instance_certificate,
        )
        .await
        .expect("guest deploy failed");

    if with_cert {
        // The receiving node re-verifies the driver's instance certificate
        // on every undo, and cannot do that without the master's anchor.
        RegistryClient::new(false, Some(node.registry_url.clone()))
            .publish_master_anchor(&service_id, vec![], None, master, true)
            .await
            .expect("failed to publish the caller master's anchor");
    }

    let mechanisms =
        node.substrate_client.lookup().await.expect("node lookup failed").info.mechanisms;
    publish_endpoint(&service_id, node.did(), mechanisms, master, &node.registry_url).await;
    service_id
}

/// Drives the driver's own `begin-workflow` export -- real guest code
/// calling `syneroym:proxy/saga::begin`, not a Rust-level fake.
async fn begin_workflow(node: &Node, driver_did: &str, deadline_secs: u64) -> String {
    let mut client = SyneroymClient::new_with_identity(
        driver_did.to_string(),
        node.registry_url.clone(),
        Identity::generate().unwrap(),
    );
    client.connect().await.expect("connect failed");
    let res = client
        .request(
            test_constants::SAGA_TEST_DRIVER_INTERFACE,
            "begin-workflow",
            json!({"deadline-secs": deadline_secs}),
        )
        .await
        .expect("begin-workflow failed")
        .result;
    client.shutdown().await.ok();
    res.as_str().expect("begin-workflow must return a saga id string").to_string()
}

async fn add_step(node: &Node, driver_did: &str, saga_id: &str, peer_did: &str, item: &str) {
    let mut client = SyneroymClient::new_with_identity(
        driver_did.to_string(),
        node.registry_url.clone(),
        Identity::generate().unwrap(),
    );
    client.connect().await.expect("connect failed");
    client
        .request(
            test_constants::SAGA_TEST_DRIVER_INTERFACE,
            "add-step",
            json!({"saga": saga_id, "peer": peer_did, "item": item}),
        )
        .await
        .expect("add-step failed");
    client.shutdown().await.ok();
}

async fn finish_workflow(node: &Node, driver_did: &str, saga_id: &str, outcome: &str) {
    let mut client = SyneroymClient::new_with_identity(
        driver_did.to_string(),
        node.registry_url.clone(),
        Identity::generate().unwrap(),
    );
    client.connect().await.expect("connect failed");
    client
        .request(
            test_constants::SAGA_TEST_DRIVER_INTERFACE,
            "finish-workflow",
            json!({"saga": saga_id, "outcome": outcome}),
        )
        .await
        .expect("finish-workflow failed");
    client.shutdown().await.ok();
}

/// Reads the participant's own audit log through an ordinary guest call --
/// deliberately not through the operator surface, so this witnesses the
/// participant's own state rather than the saga log's.
async fn read_ledger(node: &Node, participant_did: &str) -> String {
    let mut client = SyneroymClient::new_with_identity(
        participant_did.to_string(),
        node.registry_url.clone(),
        Identity::generate().unwrap(),
    );
    client.connect().await.expect("connect failed");
    let res = client
        .request(test_constants::SAGA_TEST_PARTICIPANT_INTERFACE, "ledger", Value::Null)
        .await
        .expect("ledger read failed")
        .result;
    client.shutdown().await.ok();
    res.as_str().expect("ledger must return a string").to_string()
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
        time::sleep(Duration::from_millis(500)).await;
    }
    false
}

/// This e2e must fail, not skip, on a missing artifact -- a passing run
/// against a missing component is the exact failure the operator surface
/// exists to prevent.
fn fail_if_fixture_missing() {
    assert!(
        test_constants::saga_test_wasm_path().exists(),
        "test-components/saga-test's wasm artifact is not built. Run, from that directory: `cargo \
         component build --release --target wasm32-wasip2`"
    );
}

fn artifact() -> Vec<u8> {
    fs::read(test_constants::saga_test_wasm_path()).expect(
        "saga-test wasm artifact must exist; fail_if_fixture_missing should have caught this",
    )
}

/// The order the reverse walk undoes steps in, and the whole thing running
/// against two real, independently-crashable nodes -- the sequence no
/// in-process test proves.
#[tokio::test]
async fn a_failed_workflow_is_undone_in_reverse_order() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    fail_if_fixture_missing();
    let _ = ring::default_provider().install_default();
    let wasm = artifact();
    let (a_iroh, a_reg, a_gw, b_iroh, b_reg, b_gw) = REVERSE_WALK_PORTS;

    let driver_dir = tempfile::tempdir().unwrap();
    let participant_dir = tempfile::tempdir().unwrap();
    let owner = Identity::generate().unwrap();

    let mut node_a = Node::boot(
        driver_dir.path().to_path_buf(),
        a_iroh,
        a_reg,
        a_gw,
        None,
        &owner,
        Some(fast_saga_role()),
    )
    .await;
    let shared_registry = node_a.registry_url.clone();

    let mut node_b = Node::boot(
        participant_dir.path().to_path_buf(),
        b_iroh,
        b_reg,
        b_gw,
        Some(shared_registry.clone()),
        &owner,
        None,
    )
    .await;

    let participant_master = Identity::generate().unwrap();
    let participant_did = deploy_guest(&mut node_b, &participant_master, wasm.clone(), false).await;

    let driver_master = Identity::generate().unwrap();
    let driver_did = deploy_guest(&mut node_a, &driver_master, wasm, true).await;

    let saga_id = begin_workflow(&node_a, &driver_did, 3600).await;
    add_step(&node_a, &driver_did, &saga_id, &participant_did, "a").await;
    add_step(&node_a, &driver_did, &saga_id, &participant_did, "b").await;
    assert_eq!(read_ledger(&node_b, &participant_did).await, "reserve:a, reserve:b");
    finish_workflow(&node_a, &driver_did, &saga_id, "compensate").await;

    // `compensate` itself returns before the walk runs -- proved directly,
    // with mocked timing, by the in-process router test
    // `the_walk_undoes_the_newest_step_first`. With a 1s sweep tick this
    // window is too narrow for a real two-node e2e to observe reliably, so
    // this asserts only the property unique to a real restart-free run: the
    // saga is *no longer* `open` once `compensate` returns.
    let status = node_a
        .substrate_client
        .sagas(driver_did.clone())
        .await
        .expect("sagas failed")
        .into_iter()
        .find(|s| s.saga_id == saga_id)
        .expect("the saga must be listed");
    assert_ne!(status.state, SagaState::Open);

    assert!(
        wait_until(Duration::from_secs(60), || async {
            read_ledger(&node_b, &participant_did).await == "reserve:a, reserve:b, undo:b, undo:a"
        })
        .await,
        "the walk must undo the newest step first; ledger is now {:?}",
        read_ledger(&node_b, &participant_did).await
    );

    // `node_a`'s connection sat idle through the entire 60s `wait_until`
    // poll loop above (which only drives `node_b`'s client), long enough
    // under CI's scheduling pressure for the peer to abandon that idle
    // path ("no viable network path exists: last path abandoned by peer";
    // same root cause fixed throughout this crate's e2e tests). Recover by
    // explicit shutdown→reconnect before one retry.
    let sagas_a = crate::call_with_reconnect!(
        node_a.substrate_client,
        node_a.substrate_client.sagas(driver_did.clone()).await
    );
    let status =
        sagas_a.into_iter().find(|s| s.saga_id == saga_id).expect("the saga must still be listed");
    assert_eq!(status.state, SagaState::Compensated);

    // A second sweep changes nothing: the walk does not re-run.
    time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        read_ledger(&node_b, &participant_did).await,
        "reserve:a, reserve:b, undo:b, undo:a"
    );

    node_b.teardown().await;
    node_a.teardown().await;
}

/// The crash case: nothing but the deadline can ever unwind a saga whose
/// driver died mid-workflow, because a guest does not exist between calls.
/// Also the locked-vault property in both directions: nothing is
/// compensated before the KEK is injected, and the sweep says so rather
/// than silently skipping.
#[tokio::test]
async fn a_workflow_abandoned_across_a_restart_is_compensated_by_its_deadline() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    fail_if_fixture_missing();
    let _ = ring::default_provider().install_default();
    let wasm = artifact();
    let (a_iroh, a_reg, a_gw, b_iroh, b_reg, b_gw) = DEADLINE_PORTS;

    let driver_dir = tempfile::tempdir().unwrap();
    let participant_dir = tempfile::tempdir().unwrap();
    let owner = Identity::generate().unwrap();

    let mut node_a = Node::boot(
        driver_dir.path().to_path_buf(),
        a_iroh,
        a_reg,
        a_gw,
        None,
        &owner,
        Some(fast_saga_role()),
    )
    .await;
    let shared_registry = node_a.registry_url.clone();

    let mut node_b = Node::boot(
        participant_dir.path().to_path_buf(),
        b_iroh,
        b_reg,
        b_gw,
        Some(shared_registry.clone()),
        &owner,
        None,
    )
    .await;
    let participant_master = Identity::generate().unwrap();
    let participant_did = deploy_guest(&mut node_b, &participant_master, wasm.clone(), false).await;

    let driver_master = Identity::generate().unwrap();
    let driver_did = deploy_guest(&mut node_a, &driver_master, wasm, true).await;

    // A short deadline: long enough to add the one step, short enough that
    // the test does not have to wait long for it to pass.
    let saga_id = begin_workflow(&node_a, &driver_did, 20).await;
    add_step(&node_a, &driver_did, &saga_id, &participant_did, "a").await;
    assert_eq!(read_ledger(&node_b, &participant_did).await, "reserve:a");
    let status = node_a
        .substrate_client
        .sagas(driver_did.clone())
        .await
        .expect("sagas failed")
        .into_iter()
        .find(|s| s.saga_id == saga_id)
        .expect("the saga must be listed");
    assert_eq!(status.state, SagaState::Open);

    // Tear down node A and restart it locked -- the step no in-process test
    // can express, and the reason the log is durable at all.
    node_a.teardown().await;
    // Same ports, not a fresh block: node A hosts the registry itself (like
    // `proxy_outbox_e2e.rs`'s own node A), and `None` here means it brings
    // its own back up at the same address rather than depending on some
    // other node's. The community registry keeps its records **in
    // memory**, so this restart empties it regardless of the address being
    // unchanged -- node B's record and the driver's master anchor are
    // republished below, the same requirement `proxy_outbox_e2e.rs`
    // records for its own node A restart.
    let mut node_a = Node::boot_locked(
        driver_dir.path().to_path_buf(),
        a_iroh,
        a_reg,
        a_gw,
        None,
        &owner,
        Some(fast_saga_role()),
    )
    .await;

    // The community registry node A hosts keeps its records in memory, so
    // its own restart empties it -- republished here for the same reason
    // `proxy_outbox_e2e.rs`'s restart case does. Neither call needs the
    // KEK, so this happens before `unlock()`.
    //
    // `node_b`'s connection has sat idle since its own `deploy_guest` call,
    // through node A's `teardown` and full `boot_locked` reboot -- long
    // enough under CI's scheduling pressure for the peer to abandon that
    // idle path ("no viable network path exists: last path abandoned by
    // peer"; same root cause fixed throughout this crate's e2e tests).
    // Recover by explicit shutdown→reconnect before one retry.
    let node_b_info = crate::call_with_reconnect!(
        node_b.substrate_client,
        node_b.substrate_client.lookup().await
    )
    .info
    .mechanisms;
    publish_endpoint(
        &participant_did,
        node_b.did(),
        node_b_info,
        &participant_master,
        &shared_registry,
    )
    .await;
    RegistryClient::new(false, Some(shared_registry.clone()))
        .publish_master_anchor(&driver_did, vec![], None, &driver_master, true)
        .await
        .expect("failed to republish the driver master's anchor after the restart");

    // Past the deadline, still locked: nothing is compensated, and the
    // operator verb answers with an error naming the locked vault rather
    // than an empty (and ambiguous) list.
    time::sleep(Duration::from_secs(25)).await;
    assert_eq!(
        read_ledger(&node_b, &participant_did).await,
        "reserve:a",
        "a locked substrate must compensate nothing"
    );
    let locked_answer = node_a.substrate_client.sagas(driver_did.clone()).await;
    assert!(
        locked_answer.is_err(),
        "sagas must answer with an error while the vault is locked, not an empty list -- a locked \
         node and an idle one must not look the same"
    );

    // Unlocked: the sagas verb answers again, and the abandoned saga is
    // compensated on its own, with no guest involved at any point after the
    // restart.
    node_a.unlock().await;
    assert!(
        wait_until(Duration::from_secs(60), || async {
            read_ledger(&node_b, &participant_did).await == "reserve:a, undo:a"
        })
        .await,
        "the saga must be compensated by its own deadline once the vault unlocks; ledger is now \
         {:?}",
        read_ledger(&node_b, &participant_did).await
    );
    // `node_a`'s connection sat idle through the 60s `wait_until` poll loop
    // above (which only drives `node_b`'s client), long enough under CI's
    // scheduling pressure for the peer to abandon that idle path. Recover
    // by explicit shutdown→reconnect before one retry.
    let sagas_a = crate::call_with_reconnect!(
        node_a.substrate_client,
        node_a.substrate_client.sagas(driver_did.clone()).await
    );
    let status = sagas_a
        .into_iter()
        .find(|s| s.saga_id == saga_id)
        .expect("the saga must have survived the restart under the same id");
    assert_eq!(status.state, SagaState::Compensated);

    node_b.teardown().await;
    node_a.teardown().await;
}
