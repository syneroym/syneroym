#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! M05B Slice B1's own reference scenario (task.md), end to end over two
//! real `syneroym-substrate` instances: `backend` (the dependency) on
//! `managed-a`, `frontend` (the dependent) on `managed-b`. Both plain TCP
//! services -- this scenario is about durable *delivery* of a binding
//! write, not about a live dependency call resolving through one, so it
//! needs no WASM fixture (unlike M05A's own `reference_scenario_e2e.rs`).
//!
//! Steps, matching task.md's reference scenario exactly:
//!
//! 1. Deploy, converged.
//! 2. Stop `managed-b` (`frontend`'s host).
//! 3. Scale `backend` on `managed-a` -- a membership change that makes a
//!    binding push to `frontend` (on the now-offline `managed-b`) due.
//! 4. The synchronous push fails: `Degraded`, unconverged, and `submit` itself
//!    already returned rather than retrying forever in-process.
//! 5. Restart the supervisor process. The queued item survives.
//! 6. Bring `managed-b` back, same identity.
//! 7. Within one worker tick, convergence resumes and the alert the failure
//!    raised clears.
//! 8. (separate test) Take `managed-b` down permanently and exhaust the attempt
//!    budget: the item lands in the DLQ, an alert is raised, `dead-letters`
//!    lists it, and `replay` re-queues it rather than executing it inline.
//!
//! `Node`/the reboot-the-same-identity pattern is copied from
//! `supervisor_loop_e2e.rs`. `queue_*` are configured far below their
//! production defaults (§0.12's ~10-hour attempt budget) so step 8
//! completes in seconds rather than hours -- that arithmetic is pinned by
//! `syneroym-async-queue`'s own unit tests, not re-proven here; this file
//! proves the *sequence*.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};

use rustls::crypto::ring;
use semver::Version;
use serde_json::{Map, Value, json};
use syneroym_app_orchestration::{
    LocalFilesystemCatalog, compile,
    models::{
        AppBlueprintId, AppInstanceId, LogicalServiceName, PlacementSelector, ServiceConfig,
        ServiceSpec, ServiceType, SubstrateAlias, SynAppManifest,
    },
};
use syneroym_app_supervisor::inventory::SupervisorInventoryEntry;
use syneroym_core::config::{
    ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole, IrohParentConfig, LogTarget,
    ServiceRegistryRole, SubstrateConfig, SupervisorRole,
};
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};
use syneroym_sdk::SyneroymClient;
use syneroym_substrate::identity;
use tokio::{
    sync::{Mutex, mpsc, mpsc::Sender},
    task::JoinHandle,
    time,
};

#[derive(Clone, Copy)]
struct PortBlock {
    supervisor_iroh: u16,
    supervisor_registry: u16,
    supervisor_gateway: u16,
    managed_a_iroh: u16,
    managed_a_registry: u16,
    managed_a_gateway: u16,
    managed_b_iroh: u16,
    managed_b_registry: u16,
    managed_b_gateway: u16,
}

// The highest block claimed in this directory before this file is
// `app_instance_identity_e2e.rs`'s single-node 13_000-13_102. Continuing
// one hundred-port block past that; the second test offsets by 500 more so
// the two tests in this file never collide with each other.
const PORTS: PortBlock = PortBlock {
    supervisor_iroh: 13_200,
    supervisor_registry: 13_201,
    supervisor_gateway: 13_202,
    managed_a_iroh: 13_300,
    managed_a_registry: 13_301,
    managed_a_gateway: 13_302,
    managed_b_iroh: 13_400,
    managed_b_registry: 13_401,
    managed_b_gateway: 13_402,
};

/// Not sharing a port block keeps the two tests here from colliding, but
/// they still each boot three full substrate instances (real iroh QUIC
/// socket, self-hosted relay, mainline DHT, wasmtime), and running that
/// many concurrently starves the CI runner's CPU badly enough that iroh's
/// QUIC path validation times out with "no viable network path exists"
/// even though nothing is actually broken. Serializing full-substrate-
/// instance lifetimes within this binary (same fix as `tests/common/mod.rs`'s
/// `SUBSTRATE_TEST_LOCK`) avoids it.
static SUBSTRATE_TEST_LOCK: Mutex<()> = Mutex::const_new(());

const MANAGED_A_ALIAS: &str = "managed-a";
const MANAGED_B_ALIAS: &str = "managed-b";
const INSTANCE_ID: &str = "b1-ref-scenario-inst";

/// Copied from `supervisor_loop_e2e.rs`: a full, independently-identified
/// `syneroym-substrate` instance booted from a caller-owned directory, so
/// the same identity (supervisor or managed) can be torn down and rebooted
/// later in the same test.
struct Node {
    registry_url: String,
    substrate_client: SyneroymClient,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
}

impl Node {
    #[allow(clippy::too_many_arguments)]
    async fn boot(
        base_path: PathBuf,
        iroh_port: u16,
        registry_port: u16,
        gateway_port: u16,
        shared_registry_url: Option<String>,
        shared_relay_url: Option<String>,
        owner: &Identity,
        supervisor: Option<SupervisorRole>,
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
        let relay_url = shared_relay_url.unwrap_or_else(|| format!("http://localhost:{iroh_port}"));
        config.parent_coordinator.iroh = Some(IrohParentConfig { url: relay_url });
        config.roles.client_gateway = Some(ClientGatewayRole { http_port: gateway_port });
        config.iam.admin_ucan_root = Some(substrate::derive_did_key(&owner.public_key()));
        config.roles.supervisor = supervisor;

        let substrate_identity_state =
            identity::setup_substrate_identity(&config.identity, &config.app_data_dir)
                .expect("failed to setup identity");
        let substrate_service_id = substrate_identity_state.did.clone();

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
            substrate_service_id.clone(),
            effective_registry_url.clone(),
            Identity::from_bytes(&owner.to_bytes()),
        );
        substrate_client
            .wait_for_ready(Duration::from_secs(30))
            .await
            .expect("substrate did not become available in time");
        substrate_client.inject_kek(hex::encode([0xabu8; 32])).await.expect("inject_kek failed");

        Self {
            registry_url: effective_registry_url,
            substrate_client,
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

/// Fast, test-only queue knobs (M05B B1). The production defaults give a
/// ~10-hour attempt budget (`slice-b1-implementation-plan.md` §0.12) --
/// deliberately not reproduced here; this scenario needs the *sequence* to
/// happen, not the real window.
/// `poll_interval_secs` is a caller-chosen parameter, not a fixed constant:
/// the resident loop's own pass is what makes the *initial* scale-out push
/// due in the first place (`submit`/`force-reconcile` are both all-or-
/// nothing across every placed alias via `build_clients`, so neither can
/// land backend's redeploy while managed-b is down -- only the loop's own
/// best-effort `connect_best_effort` can). Discovery needs a short poll
/// interval; task.md's own recovery budget ("within one worker tick, not
/// one poll interval") needs a long one for the *second* boot, after the
/// item is already queued, so a passing convergence cannot be a coincidence
/// of the loop also happening to retry in time.
fn supervisor_role(poll_interval_secs: u64) -> SupervisorRole {
    SupervisorRole {
        poll_interval_secs,
        db_name: "supervisor.db".to_string(),
        max_restart_attempts: 3,
        restart_backoff_secs: 30,
        alert_topic: "supervisor/alerts".to_string(),
        master_backup_dir: "master-backups".to_string(),
        queue_tick_secs: 1,
        queue_max_attempts: 3,
        queue_max_backoff_secs: 1,
        queue_visibility_timeout_secs: 5,
        queue_dlq_max_rows: 10,
        ..SupervisorRole::default()
    }
}

fn node_wide_supervisor_grant(
    node_owner: &Identity,
    grantee_did: &str,
    node_did: &str,
) -> CapabilityToken {
    let resource = ResourceUri::substrate(node_did);
    CapabilityToken::issue(
        node_owner,
        grantee_did,
        [Ability::ORCHESTRATOR_DEPLOY, Ability::ORCHESTRATOR_STATUS]
            .into_iter()
            .map(|a| Capability {
                with: resource.clone(),
                can: Ability(a.to_string()),
                caveats: None,
            })
            .collect(),
        Map::new(),
        3600,
        vec![],
    )
    .expect("issue node-wide supervisor grant")
}

/// `backend` (the dependency) on `managed-a`, scaled to `backend_replicas`;
/// `frontend` (the dependent) on `managed-b`, depending on it. Plain TCP
/// services -- this scenario is about durable delivery of the binding
/// write, not about a live dependency call resolving through it.
fn manifest(backend_replicas: u32) -> SynAppManifest {
    let mut services = BTreeMap::new();
    services.insert(
        LogicalServiceName::new("backend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41801".to_string(),
                hash: None,
                interfaces: vec![],
                env: BTreeMap::new(),
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: Default::default(),
                fdae: None,
                health_check: None,
            },
            depends_on: vec![],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_A_ALIAS))),
            replicas: backend_replicas,
            sharding_strategy: None,
            schedule: None,
        },
    );
    services.insert(
        LogicalServiceName::new("frontend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41802".to_string(),
                hash: None,
                interfaces: vec![],
                env: BTreeMap::new(),
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: Default::default(),
                fdae: None,
                health_check: None,
            },
            depends_on: vec![LogicalServiceName::new("backend")],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_B_ALIAS))),
            replicas: 1,
            sharding_strategy: None,
            schedule: None,
        },
    );
    SynAppManifest {
        id: AppBlueprintId::new("syneroym:b1-ref-scenario"),
        version: Version::new(0, 1, 0),
        description: None,
        placement: None,
        services,
        dependencies: BTreeMap::new(),
    }
}

async fn compiled_plan_json(backend_replicas: u32) -> String {
    let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
    let compiled = compile(AppInstanceId::new(INSTANCE_ID), &manifest(backend_replicas), &catalog)
        .await
        .unwrap();
    compiled.plans.last().unwrap().to_json().unwrap()
}

fn submission(plan_json: String, inventory_json: String, generation: u64) -> Value {
    json!([{
        "app_instance_id": INSTANCE_ID,
        "plan_json": plan_json,
        "inventory_json": inventory_json,
        "generation": generation,
    }])
}

fn str_field<'a>(v: &'a Value, field: &str) -> Option<&'a str> {
    v.get(field).and_then(|f| f.as_str())
}

/// The very first `submit` after booting every node needs the supervisor
/// to reach *both* freshly-started managed substrates at once --
/// `connected_client`'s own 10s budget (`MANAGED_SUBSTRATE_CONNECT_
/// TIMEOUT`) can be tighter than DHT/relay propagation for an endpoint
/// record published moments earlier. Every later call in this file (after
/// the first pass has already reached each substrate once) needs no such
/// retry.
async fn submit_with_retry(supervisor_node: &Node, params: Value) {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        match supervisor_node.substrate_client.request("supervisor", "submit", params.clone()).await
        {
            Ok(_) => return,
            Err(e) if Instant::now() < deadline => {
                tracing::debug!(error = %e, "initial submit not yet reachable, retrying");
                time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => panic!("submit failed after retrying for 180s: {e}"),
        }
    }
}

async fn supervisor_status(supervisor_node: &Node) -> Value {
    supervisor_node
        .substrate_client
        .request("supervisor", "status", json!([INSTANCE_ID]))
        .await
        .expect("status failed")
        .result
}

/// `frontend`'s convergence row for its `backend` dependency, off the
/// supervisor's `status` -- `None` until the first push lands at all.
fn frontend_binding(status: &Value) -> Option<&Value> {
    status.get("bindings").and_then(Value::as_array).into_iter().flatten().find(|b| {
        str_field(b, "dependent_logical_ref").is_some_and(|r| r.ends_with("/frontend#0"))
            && str_field(b, "dependency_name") == Some("backend")
    })
}

fn is_converged(status: &Value) -> bool {
    frontend_binding(status).and_then(|b| b.get("converged")).and_then(Value::as_bool) == Some(true)
}

/// The ids of every item this instance's outbox currently holds against
/// `substrate_did` -- M05B B1 review finding 13: the reference scenario's
/// own steps 4/5/7 ask to assert the item is "in the outbox"/"still
/// queued"/"the outbox is empty", which only this verb (not `alerts` or
/// `is_converged`) can answer directly.
async fn outbox_item_ids(supervisor_node: &Node, substrate_did: &str) -> Vec<u64> {
    let items = supervisor_node
        .substrate_client
        .request("supervisor", "outbox", json!([INSTANCE_ID]))
        .await
        .expect("outbox failed")
        .result;
    items
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| str_field(i, "substrate_did") == Some(substrate_did))
        .filter_map(|i| i.get("id").and_then(Value::as_u64))
        .collect()
}

async fn active_alert_kinds(supervisor_node: &Node, substrate_did: &str) -> BTreeSet<String> {
    let alerts = supervisor_node
        .substrate_client
        .request("supervisor", "alerts", json!([INSTANCE_ID, false]))
        .await
        .expect("alerts failed")
        .result;
    alerts
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| str_field(a, "substrate_did") == Some(substrate_did))
        .filter_map(|a| str_field(a, "kind").map(str::to_string))
        .collect()
}

#[tokio::test]
async fn a_binding_push_to_an_offline_substrate_converges_after_it_returns() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let _ = ring::default_provider().install_default();

    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();

    let supervisor_dir = tempfile::tempdir().expect("failed to create temp dir");
    let mut supervisor_node = Node::boot(
        supervisor_dir.path().to_path_buf(),
        PORTS.supervisor_iroh,
        PORTS.supervisor_registry,
        PORTS.supervisor_gateway,
        None,
        None,
        &supervisor_owner,
        // Short for this first boot: the resident loop's own pass is what
        // discovers step 3's scale-out diff and attempts (and fails, and
        // enqueues) the push to frontend while managed-b is down.
        Some(supervisor_role(3)),
    )
    .await;
    let shared_registry = supervisor_node.registry_url.clone();
    let shared_relay = format!("http://localhost:{}", PORTS.supervisor_iroh);

    let managed_a_dir = tempfile::tempdir().expect("failed to create temp dir");
    let managed_a = Node::boot(
        managed_a_dir.path().to_path_buf(),
        PORTS.managed_a_iroh,
        PORTS.managed_a_registry,
        PORTS.managed_a_gateway,
        Some(shared_registry.clone()),
        Some(shared_relay.clone()),
        &managed_owner,
        None,
    )
    .await;

    let managed_b_dir = tempfile::tempdir().expect("failed to create temp dir");
    let mut managed_b = Node::boot(
        managed_b_dir.path().to_path_buf(),
        PORTS.managed_b_iroh,
        PORTS.managed_b_registry,
        PORTS.managed_b_gateway,
        Some(shared_registry.clone()),
        Some(shared_relay.clone()),
        &managed_owner,
        None,
    )
    .await;
    let managed_b_did = managed_b.did().to_string();
    let managed_b_registry_url = managed_b.registry_url.clone();

    let grant_a =
        node_wide_supervisor_grant(&managed_owner, supervisor_node.did(), managed_a.did());
    let grant_b = node_wide_supervisor_grant(&managed_owner, supervisor_node.did(), &managed_b_did);
    let inventory_json = serde_json::to_string(&BTreeMap::from([
        (
            MANAGED_A_ALIAS.to_string(),
            SupervisorInventoryEntry {
                did: managed_a.did().to_string(),
                api_url: Some(managed_a.registry_url.clone()),
                ucan: Some(grant_a),
            },
        ),
        (
            MANAGED_B_ALIAS.to_string(),
            SupervisorInventoryEntry {
                did: managed_b_did.clone(),
                api_url: Some(managed_b_registry_url.clone()),
                ucan: Some(grant_b),
            },
        ),
    ]))
    .unwrap();

    // Step 1: deploy, converged.
    let plan_json = compiled_plan_json(1).await;
    submit_with_retry(&supervisor_node, submission(plan_json, inventory_json.clone(), 0)).await;

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = supervisor_status(&supervisor_node).await;
        if is_converged(&status) {
            break;
        }
        assert!(Instant::now() < deadline, "initial deploy never converged: {status:?}");
        time::sleep(Duration::from_millis(300)).await;
    }

    // Step 2: managed-b (frontend's host) goes down.
    managed_b.teardown().await;

    // Step 3: scale backend on managed-a -- a membership change that makes
    // a binding push to frontend (on the now-offline managed-b) due.
    // `submit`'s own best-effort apply is all-or-nothing across every
    // targeted alias (`build_clients`), so this call itself is expected to
    // surface an error even though the desired state underneath it is
    // still durably recorded -- the same shape `supervisor_loop_e2e.rs`
    // already establishes for a partial deploy.
    let scaled_plan_json = compiled_plan_json(2).await;
    let scale_out_res = supervisor_node
        .substrate_client
        .request("supervisor", "submit", submission(scaled_plan_json, inventory_json.clone(), 0))
        .await;
    assert!(scale_out_res.is_err(), "submit with managed-b down must surface the failure");

    // Step 4: the synchronous push fails. Degraded, unconverged -- and not
    // retried forever in-process: the `submit` call above already
    // returned rather than blocking on delivery.
    //
    // Waited for directly, not inferred from `status.state == Degraded`:
    // the health sweep alone reports Degraded the moment backend's new
    // replica is merely `missing_placement` (before it has even landed on
    // managed-a), which is unrelated to -- and can precede -- the push
    // failure this step actually asserts. Only once backend#1 has landed
    // does frontend's diff become a push candidate at all.
    //
    // A generous deadline: every pass while managed-b is down pays its own
    // `MANAGED_SUBSTRATE_CONNECT_TIMEOUT` (10s) trying to reach it, and
    // `MissedTickBehavior::Skip` means a pass that overruns
    // `poll_interval_secs` (3s) drops the tick it overran rather than
    // queueing a burst -- so the loop's real cadence here is close to 10s
    // per pass, not 3s, and landing backend's redeploy plus attempting
    // frontend's push can take a few such passes.
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if active_alert_kinds(&supervisor_node, &managed_b_did).await.contains("BINDING_CONFLICT") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "a push that cannot reach its target must still raise BindingConflict (matrix row 11)"
        );
        time::sleep(Duration::from_millis(300)).await;
    }
    let status = supervisor_status(&supervisor_node).await;
    assert!(
        str_field(&status, "state") == Some("Degraded") && !is_converged(&status),
        "once the push has failed the instance must report Degraded, unconverged: {status:?}"
    );
    let outbox_ids_before_restart = outbox_item_ids(&supervisor_node, &managed_b_did).await;
    assert_eq!(
        outbox_ids_before_restart.len(),
        1,
        "task.md step 4: the failed push must be in the outbox, not merely inferred from alerts"
    );

    // Step 5: restart the supervisor process. The queued item survives --
    // this is the step no in-process retry can.
    supervisor_node.teardown().await;
    supervisor_node = Node::boot(
        supervisor_dir.path().to_path_buf(),
        PORTS.supervisor_iroh,
        PORTS.supervisor_registry,
        PORTS.supervisor_gateway,
        None,
        None,
        &supervisor_owner,
        // Long for this second boot: task.md's own recovery budget is
        // "within one worker tick, not one poll interval" -- set far above
        // `queue_tick_secs` (1s) so the loop's own next pass is nowhere
        // near step 7's window, and the convergence it asserts there can
        // only have come from the queue worker.
        Some(supervisor_role(3600)),
    )
    .await;
    let status = supervisor_status(&supervisor_node).await;
    assert!(
        !is_converged(&status),
        "the item must still be queued right after restart: {status:?}"
    );
    assert_eq!(
        outbox_item_ids(&supervisor_node, &managed_b_did).await,
        outbox_ids_before_restart,
        "task.md step 5: the exact same item must still be queued across the restart, not a \
         different or duplicated one"
    );

    // Step 6: bring managed-b back, same identity.
    managed_b = Node::boot(
        managed_b_dir.path().to_path_buf(),
        PORTS.managed_b_iroh,
        PORTS.managed_b_registry,
        PORTS.managed_b_gateway,
        Some(shared_registry),
        Some(shared_relay),
        &managed_owner,
        None,
    )
    .await;
    assert_eq!(managed_b.did(), managed_b_did, "the rebooted node must keep its identity");

    // Step 7: within one worker tick (1s here; task.md's own budget is
    // "not one poll interval", 3600s in this test), convergence resumes
    // and the queue's own delivery clears the alert it raised. The
    // deadline is generous (matching step 4's) because it bounds a freshly
    // rebooted node's own relay/DHT re-announce, not the worker tick
    // itself -- the worker retries every `queue_tick_secs` (1s) regardless,
    // but each attempt still pays its own connect timeout until managed-b
    // is genuinely reachable again.
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let status = supervisor_status(&supervisor_node).await;
        if is_converged(&status) && str_field(&status, "state") == Some("Active") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "recovery after managed-b returned took longer than one worker tick: {status:?}"
        );
        time::sleep(Duration::from_millis(300)).await;
    }
    assert!(
        active_alert_kinds(&supervisor_node, &managed_b_did).await.is_empty(),
        "the earlier BindingConflict must clear once delivery converges"
    );
    assert!(
        outbox_item_ids(&supervisor_node, &managed_b_did).await.is_empty(),
        "task.md step 7: the outbox must be empty once delivery converges, not merely the alert \
         cleared"
    );

    supervisor_node.teardown().await;
    managed_a.teardown().await;
    managed_b.teardown().await;
}

/// Step 8: a substrate that never comes back exhausts the outbox's attempt
/// budget, and the item becomes visible and replayable rather than
/// silently lost.
#[tokio::test]
async fn a_permanently_unreachable_substrate_lands_in_the_dlq_and_replays() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let _ = ring::default_provider().install_default();

    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();

    let supervisor_dir = tempfile::tempdir().expect("failed to create temp dir");
    let supervisor_node = Node::boot(
        supervisor_dir.path().to_path_buf(),
        PORTS.supervisor_iroh + 500,
        PORTS.supervisor_registry + 500,
        PORTS.supervisor_gateway + 500,
        None,
        None,
        &supervisor_owner,
        // Short throughout, the same reason test 1's first boot is: the
        // resident loop's pass is what discovers the scale-out diff and
        // attempts (and fails, and enqueues) the push while managed-b is
        // down. DLQ exhaustion itself is driven by the queue worker's own
        // tick, not by this interval.
        Some(supervisor_role(3)),
    )
    .await;
    let shared_registry = supervisor_node.registry_url.clone();
    let shared_relay = format!("http://localhost:{}", PORTS.supervisor_iroh + 500);

    let managed_a_dir = tempfile::tempdir().expect("failed to create temp dir");
    let managed_a = Node::boot(
        managed_a_dir.path().to_path_buf(),
        PORTS.managed_a_iroh + 500,
        PORTS.managed_a_registry + 500,
        PORTS.managed_a_gateway + 500,
        Some(shared_registry.clone()),
        Some(shared_relay.clone()),
        &managed_owner,
        None,
    )
    .await;

    let managed_b_dir = tempfile::tempdir().expect("failed to create temp dir");
    let managed_b = Node::boot(
        managed_b_dir.path().to_path_buf(),
        PORTS.managed_b_iroh + 500,
        PORTS.managed_b_registry + 500,
        PORTS.managed_b_gateway + 500,
        Some(shared_registry.clone()),
        Some(shared_relay.clone()),
        &managed_owner,
        None,
    )
    .await;
    let managed_b_did = managed_b.did().to_string();
    let managed_b_registry_url = managed_b.registry_url.clone();

    let grant_a =
        node_wide_supervisor_grant(&managed_owner, supervisor_node.did(), managed_a.did());
    let grant_b = node_wide_supervisor_grant(&managed_owner, supervisor_node.did(), &managed_b_did);
    let inventory_json = serde_json::to_string(&BTreeMap::from([
        (
            MANAGED_A_ALIAS.to_string(),
            SupervisorInventoryEntry {
                did: managed_a.did().to_string(),
                api_url: Some(managed_a.registry_url.clone()),
                ucan: Some(grant_a),
            },
        ),
        (
            MANAGED_B_ALIAS.to_string(),
            SupervisorInventoryEntry {
                did: managed_b_did.clone(),
                api_url: Some(managed_b_registry_url.clone()),
                ucan: Some(grant_b),
            },
        ),
    ]))
    .unwrap();

    let plan_json = compiled_plan_json(1).await;
    submit_with_retry(&supervisor_node, submission(plan_json, inventory_json.clone(), 0)).await;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if is_converged(&supervisor_status(&supervisor_node).await) {
            break;
        }
        assert!(Instant::now() < deadline, "initial deploy never converged");
        time::sleep(Duration::from_millis(300)).await;
    }

    // managed-b goes down permanently -- unlike the first test, it never
    // comes back.
    managed_b.teardown().await;

    // Expected to error, the same all-or-nothing reason as the first
    // test's step 3 -- the desired state underneath it is still recorded.
    let scaled_plan_json = compiled_plan_json(2).await;
    let scale_out_res = supervisor_node
        .substrate_client
        .request("supervisor", "submit", submission(scaled_plan_json, inventory_json, 0))
        .await;
    assert!(scale_out_res.is_err(), "submit with managed-b down must surface the failure");

    // The test role's queue_max_attempts (3) and queue_max_backoff_secs
    // (1) put the item in the DLQ well inside this deadline, not task.md's
    // own ~10-hour production window (pinned separately by
    // syneroym-async-queue's own unit tests) -- but each of those 3
    // attempts still pays a real `MANAGED_SUBSTRATE_CONNECT_TIMEOUT` (10s)
    // against a genuinely offline node, on top of the resident loop's own
    // pass first discovering the diff and enqueueing it at all. The two
    // take turns rather than racing (D-B1-14's `instance_lock`), so a slow
    // pass and a slow delivery attempt do not overlap -- roughly doubling
    // the real wall-clock cost of each round relative to either alone.
    let deadline = Instant::now() + Duration::from_secs(280);
    let dead_letters = loop {
        let rows = supervisor_node
            .substrate_client
            .request("supervisor", "dead-letters", json!([INSTANCE_ID]))
            .await
            .expect("dead-letters failed")
            .result;
        let rows = rows.as_array().cloned().unwrap_or_default();
        if !rows.is_empty() {
            break rows;
        }
        assert!(Instant::now() < deadline, "the item never reached the DLQ");
        time::sleep(Duration::from_millis(300)).await;
    };
    assert_eq!(dead_letters.len(), 1, "{dead_letters:?}");
    assert!(
        active_alert_kinds(&supervisor_node, &managed_b_did).await.contains("DELIVERY_EXHAUSTED"),
        "an exhausted delivery budget must raise an alert, not fail silently"
    );

    // `replay` re-queues rather than executing inline: the RPC call itself
    // returns promptly, well before this permanently-down substrate could
    // ever actually answer.
    let dead_letter_id = dead_letters[0].get("id").and_then(Value::as_u64).expect("id");
    time::timeout(
        Duration::from_secs(5),
        supervisor_node.substrate_client.request(
            "supervisor",
            "replay",
            json!([INSTANCE_ID, dead_letter_id]),
        ),
    )
    .await
    .expect("replay must not block on delivery")
    .expect("replay failed");

    // managed-b is still down, so the replayed item fails again and
    // returns to the DLQ (D-B1-7) -- with its attempt history intact, not
    // a fresh budget. Still generous, the same reason the first wait
    // above is: a real connect timeout per attempt, serialized against the
    // resident loop's own pass via `instance_lock`.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let rows = supervisor_node
            .substrate_client
            .request("supervisor", "dead-letters", json!([INSTANCE_ID]))
            .await
            .expect("dead-letters failed")
            .result;
        let rows = rows.as_array().cloned().unwrap_or_default();
        if rows.len() == 1 {
            break;
        }
        assert!(Instant::now() < deadline, "the replayed item never returned to the DLQ");
        time::sleep(Duration::from_millis(300)).await;
    }

    // managed-b was already torn down above and never rebooted -- this
    // test's whole point is that it stays permanently unreachable.
    supervisor_node.teardown().await;
    managed_a.teardown().await;
}
