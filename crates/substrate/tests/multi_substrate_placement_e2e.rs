#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Multi-substrate placement and the substrate inventory (M05A Slice A3),
//! proven across two genuinely independent `syneroym-substrate` instances --
//! the reference scenario's `frontend`-on-A/`backend`-on-B topology, driven
//! through the same `crates/sdk::deploy` executor `roymctl app deploy` uses
//! (`sdk` is only a dev-dependency of this crate, and `roymctl` is a binary
//! that cannot be linked from a test -- so this harness calls `compile`,
//! `certify_placed_members`, and `apply_plan` directly, mirroring what
//! `roymctl`'s own per-substrate member-identity substitution does).
//!
//! `Node` is copied from `master_endpoint_record_e2e.rs`, not `tests/common`'s
//! `SubstrateTestContext` -- that harness's own module doc says two live
//! nodes at once deadlock on its setup-serialization lock.

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rustls::crypto::ring;
use semver::Version;
use serde_json::Map;
use syneroym_app_orchestration::{
    DeploymentJournal, DeploymentPlan, DeploymentState, LocalFilesystemCatalog, compile,
    models::{
        AppBlueprintId, AppInstanceId, LogicalServiceName, PlacementSelector, ServiceConfig,
        ServiceId, ServiceSpec, ServiceType, SubstrateAlias, SynAppManifest,
    },
};
use syneroym_core::{
    config::{
        ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole, IrohParentConfig, LogTarget,
        ServiceRegistryRole, SubstrateConfig,
    },
    dht_registry::{DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointType, RegistryClient},
};
use syneroym_identity::{Identity, substrate};
use syneroym_router::net_iroh::resolve_iroh_addr;
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};
use syneroym_sdk::{
    DeployManifest, NetworkEndpoint, ServiceConfig as WitServiceConfig,
    ServiceType as WitServiceType, SyneroymClient, TcpManifest,
    deploy::{self, ApplyRequest, DeployTarget, apply_plan, certify_instance},
};
use syneroym_substrate::identity;
use tempfile::TempDir;
use tokio::{
    sync::{Mutex, mpsc, mpsc::Sender},
    task::JoinHandle,
};

/// Every `#[tokio::test]` in this file runs concurrently by default (the
/// crate's other e2e files get away with one fixed port block each because
/// each file has exactly one test) -- so each of the five tests here needs
/// its own, non-overlapping port block. `coordinator_iroh`'s "/v1/info"
/// server always binds `http_bind_address`'s port + 10, so blocks are
/// spaced by 100, matching the convention every other multi-node harness in
/// this crate already follows.
#[derive(Clone, Copy)]
struct PortBlock {
    node_a_iroh: u16,
    node_a_registry: u16,
    node_a_gateway: u16,
    node_b_iroh: u16,
    node_b_registry: u16,
    node_b_gateway: u16,
}

const PORTS_DEPLOY_EACH_TO_ITS_NODE: PortBlock = PortBlock {
    node_a_iroh: 8800,
    node_a_registry: 8801,
    node_a_gateway: 8802,
    node_b_iroh: 8900,
    node_b_registry: 8901,
    node_b_gateway: 8902,
};
const PORTS_RECORD_RESOLVES_TO_OWN_SUBSTRATE: PortBlock = PortBlock {
    node_a_iroh: 9000,
    node_a_registry: 9001,
    node_a_gateway: 9002,
    node_b_iroh: 9100,
    node_b_registry: 9101,
    node_b_gateway: 9102,
};
const PORTS_CERT_REJECTED_BY_ANOTHER_SUBSTRATE: PortBlock = PortBlock {
    node_a_iroh: 9200,
    node_a_registry: 9201,
    node_a_gateway: 9202,
    node_b_iroh: 9300,
    node_b_registry: 9301,
    node_b_gateway: 9302,
};
const PORTS_UNREACHABLE_DEGRADED_AND_RETRYABLE: PortBlock = PortBlock {
    node_a_iroh: 9400,
    node_a_registry: 9401,
    node_a_gateway: 9402,
    node_b_iroh: 9500,
    node_b_registry: 9501,
    node_b_gateway: 9502,
};
const PORTS_DEPENDENTS_OWN_REGISTRY: PortBlock = PortBlock {
    node_a_iroh: 9600,
    node_a_registry: 9601,
    node_a_gateway: 9602,
    node_b_iroh: 9700,
    node_b_registry: 9701,
    node_b_gateway: 9702,
};

/// Not sharing a port block keeps the five tests here from colliding, but
/// they still each boot two full substrate instances (real iroh QUIC
/// socket, self-hosted relay, mainline DHT, wasmtime), and running that
/// many concurrently starves the CI runner's CPU badly enough that iroh's
/// QUIC path validation times out with "no viable network path exists"
/// even though nothing is actually broken. Serializing full-substrate-
/// instance lifetimes within this binary fixes it (same idea as
/// `tests/common/mod.rs`'s `SUBSTRATE_TEST_LOCK`) -- but unlike that lock,
/// which is held per-context and would deadlock a test needing two nodes
/// alive at once (see the module doc above), this one is acquired exactly
/// once per test, before either node boots, and held for the test's whole
/// body.
static SUBSTRATE_TEST_LOCK: Mutex<()> = Mutex::const_new(());

const FRONTEND_ALIAS: &str = "edge-a";
const BACKEND_ALIAS: &str = "edge-b";

/// A full, independently-identified `syneroym-substrate` instance. Mirrors
/// `master_endpoint_record_e2e.rs`'s own `Node`.
struct Node {
    base_path: std::path::PathBuf,
    registry_url: String,
    substrate_client: SyneroymClient,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
    _temp_dir: Option<TempDir>,
}

impl Node {
    /// `owner`'s DID becomes this node's `[iam].admin_ucan_root` (an unowned
    /// substrate now fails closed).
    ///
    /// `base_path`/`temp_dir` are split out from
    /// `master_endpoint_record_e2e.rs`'s own `Node::boot` (which mints a
    /// fresh temp dir internally) so the unreachable-substrate test can
    /// reboot a node under its *same* on-disk identity after a simulated
    /// outage -- one more parameter than that lint's default limit allows.
    #[allow(clippy::too_many_arguments)]
    async fn boot(
        base_path: std::path::PathBuf,
        temp_dir: Option<TempDir>,
        iroh_port: u16,
        registry_port: u16,
        gateway_port: u16,
        shared_registry_url: Option<String>,
        shared_relay_url: Option<String>,
        owner: &Identity,
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
        let relay_url = shared_relay_url.unwrap_or_else(|| format!("http://localhost:{iroh_port}"));
        config.parent_coordinator.iroh = Some(IrohParentConfig { url: relay_url });
        config.roles.client_gateway =
            Some(ClientGatewayRole { http_port: gateway_port, ..Default::default() });
        config.iam.admin_ucan_root = Some(substrate::derive_did_key(&owner.public_key()));

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

        Self {
            base_path,
            registry_url: effective_registry_url,
            substrate_client,
            shutdown_tx,
            substrate_handle,
            _temp_dir: temp_dir,
        }
    }

    /// Boots a fresh node under a brand-new temp directory, auto-cleaned on
    /// drop -- what every test but the restart one wants.
    async fn boot_fresh(
        iroh_port: u16,
        registry_port: u16,
        gateway_port: u16,
        shared_registry_url: Option<String>,
        shared_relay_url: Option<String>,
        owner: &Identity,
    ) -> Self {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let base_path = temp_dir.path().to_path_buf();
        Self::boot(
            base_path,
            Some(temp_dir),
            iroh_port,
            registry_port,
            gateway_port,
            shared_registry_url,
            shared_relay_url,
            owner,
        )
        .await
    }

    fn did(&self) -> &str {
        self.substrate_client.service_id()
    }

    /// Shuts the substrate process down but keeps its on-disk identity and
    /// data alive (leaking the temp dir deliberately), returning the path so
    /// the same node can be rebooted with the same DID -- simulating "the
    /// substrate becomes unreachable, then comes back" without losing
    /// identity continuity.
    async fn stop_and_keep_dir(mut self) -> std::path::PathBuf {
        let _ = self.substrate_client.shutdown().await;
        let _ = self.shutdown_tx.send(()).await;
        let _ = self.substrate_handle.await;
        match self._temp_dir.take() {
            Some(dir) => dir.keep(),
            None => self.base_path.clone(),
        }
    }

    async fn teardown(mut self) {
        let _ = self.substrate_client.shutdown().await;
        let _ = self.shutdown_tx.send(()).await;
        let _ = self.substrate_handle.await;
    }
}

/// An app-scoped `orchestrator/{deploy,undeploy,status}` grant, issued by
/// `node_owner`, letting `grantee_did` deploy/undeploy/list any app on
/// `node_did` -- all three abilities together (not deploy-only), matching
/// `federated_fdae_e2e.rs`'s own `app_deploy_grant`: `deploy`'s own rollback
/// path calls `undeploy` with the same caller.
fn app_deploy_grant(node_owner: &Identity, grantee_did: &str, node_did: &str) -> CapabilityToken {
    let resource = ResourceUri(format!("substrate:{node_did}/app/*"));
    CapabilityToken::issue(
        node_owner,
        grantee_did,
        [
            Ability::ORCHESTRATOR_DEPLOY,
            Ability::ORCHESTRATOR_UNDEPLOY,
            Ability::ORCHESTRATOR_STATUS,
        ]
        .into_iter()
        .map(|a| Capability { with: resource.clone(), can: Ability(a.to_string()), caveats: None })
        .collect(),
        Map::new(),
        3600,
        vec![],
    )
    .expect("issue app deploy grant")
}

fn far_future_not_after() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS)
}

/// `frontend` (placed on `edge-a`) depends on `backend` (placed on
/// `edge-b`) -- the reference scenario's own topology.
fn two_service_manifest() -> SynAppManifest {
    let mut services = BTreeMap::new();
    services.insert(
        LogicalServiceName::new("backend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41301".to_string(),
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
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(BACKEND_ALIAS))),
            replicas: 1,
            sharding_strategy: None,
            schedule: None,
        },
    );
    services.insert(
        LogicalServiceName::new("frontend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41302".to_string(),
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
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(FRONTEND_ALIAS))),
            replicas: 1,
            sharding_strategy: None,
            schedule: None,
        },
    );
    SynAppManifest {
        id: AppBlueprintId::new("syneroym:a3-test-app"),
        version: Version::new(0, 1, 0),
        description: None,
        placement: None,
        services,
        dependencies: BTreeMap::new(),
    }
}

/// Mints one member master per planned service and substitutes each
/// `service_id`/`resolved_dependencies` entry with the resolved master DID --
/// the same substitution `roymctl app deploy --mint-masters` performs
/// (`apps/roymctl/src/commands/member_identity.rs`), replicated here since
/// that logic is a CLI storage convention, not an SDK concern reachable from
/// this test.
fn mint_and_substitute_masters(
    plan: &DeploymentPlan,
) -> (DeploymentPlan, BTreeMap<ServiceId, Identity>) {
    let mut substitution: BTreeMap<ServiceId, ServiceId> = BTreeMap::new();
    let mut masters: BTreeMap<ServiceId, Identity> = BTreeMap::new();
    for svc in &plan.services {
        let master = Identity::generate().unwrap();
        let master_did = ServiceId::new(substrate::derive_did_key(&master.public_key()));
        substitution.insert(svc.service_id.clone(), master_did.clone());
        masters.insert(master_did, master);
    }

    let mut new_plan = plan.clone();
    for svc in &mut new_plan.services {
        let old_id = svc.service_id.clone();
        svc.service_id = substitution[&old_id].clone();
        svc.resolved_dependencies = svc
            .resolved_dependencies
            .iter()
            .map(|(name, members)| {
                (name.clone(), members.iter().map(|m| substitution[m].clone()).collect())
            })
            .collect();
    }
    (new_plan, masters)
}

/// Builds a client and connects it before handing out shared (`Arc`)
/// ownership -- `connect` needs `&mut self`, so it must run before the value
/// is wrapped, not after.
async fn client_for(
    node: &Node,
    operator: &Identity,
    grant: CapabilityToken,
) -> Arc<SyneroymClient> {
    let mut client = SyneroymClient::new_with_identity(
        node.did().to_string(),
        node.registry_url.clone(),
        Identity::from_bytes(&operator.to_bytes()),
    )
    .with_ucan(grant);
    client.connect().await.expect("failed to connect client");
    Arc::new(client)
}

/// Closes each client's iroh endpoint explicitly instead of leaving it to
/// `Drop`'s fire-and-forget safety net -- a client dropped that way races
/// this test's own tokio runtime shutdown and can trip iroh's "Endpoint
/// dropped without calling `Endpoint::close`" warning. Only closes a client
/// this call holds the sole `Arc` to; if something else still references
/// it, leaving it open is correct, not a leak.
async fn shutdown_clients(clients: impl IntoIterator<Item = Arc<SyneroymClient>>) {
    for mut client in clients {
        if let Some(c) = Arc::get_mut(&mut client) {
            let _ = c.shutdown().await;
        }
    }
}

/// Boots both nodes (B sharing A's registry/relay, D-A3-17's own
/// precondition), grants `operator` an app-scoped deploy grant on each,
/// injects a KEK on each (native-capability endpoints need one), and returns
/// everything a test needs to drive a deploy.
///
/// `owner` and `operator` must be **distinct** identities: `owner` becomes
/// each node's `admin_ucan_root`, which makes `has_node_wide_ability` return
/// true unconditionally for that DID -- if `operator` were the same
/// identity, its app-scoped grant would be moot and `list` would return
/// every registered endpoint (including the substrate's own native
/// orchestrator/security registration) rather than only the app it deployed,
/// silently defeating this fixture's own least-privilege setup.
async fn boot_pair(
    owner: &Identity,
    operator: &Identity,
    ports: PortBlock,
) -> (Node, Node, BTreeMap<SubstrateAlias, Arc<SyneroymClient>>) {
    let _ = ring::default_provider().install_default();

    let mut node_a = Node::boot_fresh(
        ports.node_a_iroh,
        ports.node_a_registry,
        ports.node_a_gateway,
        None,
        None,
        owner,
    )
    .await;
    let node_a_registry_url = node_a.registry_url.clone();
    let node_a_relay_url = format!("http://localhost:{}", ports.node_a_iroh);
    let node_b = Node::boot_fresh(
        ports.node_b_iroh,
        ports.node_b_registry,
        ports.node_b_gateway,
        Some(node_a_registry_url.clone()),
        Some(node_a_relay_url),
        owner,
    )
    .await;

    // `node_a`'s connection was dialed and proven live by its own
    // `wait_for_ready` during `Node::boot_fresh`, then sat idle for the
    // entire `node_b` boot that followed -- long enough under CI's scheduling
    // pressure for the peer to abandon that idle path ("no viable network
    // path exists: last path abandoned by peer"; same root cause fixed in
    // `binding_push_e2e.rs`). Recover by explicit shutdown→reconnect before
    // one retry.
    if node_a.substrate_client.inject_kek("aa".repeat(32)).await.is_err() {
        node_a
            .substrate_client
            .shutdown()
            .await
            .expect("failed to reset node A's stale connection");
        node_a.substrate_client.connect().await.expect("failed to reconnect node A");
        node_a
            .substrate_client
            .inject_kek("aa".repeat(32))
            .await
            .expect("node A inject_kek failed");
    }
    node_b.substrate_client.inject_kek("bb".repeat(32)).await.expect("node B inject_kek failed");

    let operator_did = substrate::derive_did_key(&operator.public_key());
    let client_a =
        client_for(&node_a, operator, app_deploy_grant(owner, &operator_did, node_a.did())).await;
    let client_b =
        client_for(&node_b, operator, app_deploy_grant(owner, &operator_did, node_b.did())).await;

    let clients: BTreeMap<SubstrateAlias, Arc<SyneroymClient>> = BTreeMap::from([
        (SubstrateAlias::new(FRONTEND_ALIAS), client_a),
        (SubstrateAlias::new(BACKEND_ALIAS), client_b),
    ]);

    (node_a, node_b, clients)
}

/// Certifies and signs an endpoint record for each master, mirroring
/// `deploy::certify_placed_members` but written directly against the two
/// concrete clients above (that function's own coverage lives in
/// `crates/sdk`'s unit tests; this harness only needs its result).
async fn certify_and_publish(
    plan: &DeploymentPlan,
    masters: &BTreeMap<ServiceId, Identity>,
    clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
) -> (BTreeMap<ServiceId, String>, BTreeMap<ServiceId, String>) {
    let mut certs = BTreeMap::new();
    let mut records = BTreeMap::new();
    for svc in &plan.services {
        let master = &masters[&svc.service_id];
        let alias = svc.substrate.as_ref().expect("every service in this fixture is placed");
        let client = &clients[alias];

        let cert = certify_instance(client, master, svc.service_id.as_str(), 24).await.unwrap();
        certs.insert(svc.service_id.clone(), cert.to_json().unwrap());

        let record = EndpointInfo {
            service_id: svc.service_id.to_string(),
            substrate_id: client.service_id().to_string(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: None,
            is_private: false,
            ttl: None,
            not_after: far_future_not_after(),
            generation: 0,
        }
        .sign(master)
        .unwrap();
        records.insert(svc.service_id.clone(), serde_json::to_string(&record).unwrap());
    }
    (certs, records)
}

fn deploy_targets(
    clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
) -> BTreeMap<SubstrateAlias, DeployTarget> {
    clients
        .iter()
        .map(|(alias, c)| {
            (
                alias.clone(),
                DeployTarget {
                    alias: Some(alias.clone()),
                    substrate_did: c.service_id().to_string(),
                    actor: deploy::build_actor(c.clone()),
                },
            )
        })
        .collect()
}

async fn compiled_plan() -> DeploymentPlan {
    let manifest = two_service_manifest();
    let catalog = LocalFilesystemCatalog::new(std::path::PathBuf::from("."));
    let compiled = compile(AppInstanceId::new("a3-test-inst"), &manifest, &catalog).await.unwrap();
    compiled.plans.last().unwrap().clone()
}

#[tokio::test]
async fn a_two_substrate_app_deploys_each_service_to_its_placed_node() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b, clients) =
        boot_pair(&owner, &operator, PORTS_DEPLOY_EACH_TO_ITS_NODE).await;

    let plan = compiled_plan().await;
    let (new_plan, masters) = mint_and_substitute_masters(&plan);
    let (instance_certs, registry_certs) = certify_and_publish(&new_plan, &masters, &clients).await;
    let targets = deploy_targets(&clients);

    let journal = DeploymentJournal::open_in_memory().unwrap();
    let deployment_id = journal.append(&new_plan, DeploymentState::Applying).unwrap();
    let report = apply_plan(
        ApplyRequest {
            plan: &new_plan,
            targets: &targets,
            fallback: None,
            instance_certificates: &instance_certs,
            registry_certificates: &registry_certs,
            emit_bindings: true,
            generation: 0,
            binding_epochs: &BTreeMap::new(),
        },
        &journal,
        deployment_id,
    )
    .await
    .unwrap();
    // `targets` holds a clone of each `Arc<SyneroymClient>` (via
    // `deploy::build_actor`) -- drop it now, not used again, so
    // `shutdown_clients` below sees each client at its sole `Arc` and can
    // actually close its endpoint instead of silently skipping it.
    drop(targets);
    assert!(report.is_complete(), "{:?}", report.failures);

    let client_a = &clients[&SubstrateAlias::new(FRONTEND_ALIAS)];
    let client_b = &clients[&SubstrateAlias::new(BACKEND_ALIAS)];

    let backend_svc = new_plan
        .services
        .iter()
        .find(|s| s.logical_ref.service_name.as_str() == "backend")
        .unwrap();
    let frontend_svc = new_plan
        .services
        .iter()
        .find(|s| s.logical_ref.service_name.as_str() == "frontend")
        .unwrap();

    let list_a = client_a.list_svcs().await.unwrap();
    assert_eq!(list_a.len(), 1, "{list_a:?}");
    assert_eq!(list_a[0].service_id, frontend_svc.service_id.to_string());

    let list_b = client_b.list_svcs().await.unwrap();
    assert_eq!(list_b.len(), 1, "{list_b:?}");
    assert_eq!(list_b[0].service_id, backend_svc.service_id.to_string());

    shutdown_clients(clients.into_values()).await;
    node_a.teardown().await;
    node_b.teardown().await;
}

#[tokio::test]
async fn a_placed_members_endpoint_record_resolves_to_its_own_substrate() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b, clients) =
        boot_pair(&owner, &operator, PORTS_RECORD_RESOLVES_TO_OWN_SUBSTRATE).await;

    let plan = compiled_plan().await;
    let (new_plan, masters) = mint_and_substitute_masters(&plan);
    let (instance_certs, registry_certs) = certify_and_publish(&new_plan, &masters, &clients).await;
    let targets = deploy_targets(&clients);

    let journal = DeploymentJournal::open_in_memory().unwrap();
    let deployment_id = journal.append(&new_plan, DeploymentState::Applying).unwrap();
    let report = apply_plan(
        ApplyRequest {
            plan: &new_plan,
            targets: &targets,
            fallback: None,
            instance_certificates: &instance_certs,
            registry_certificates: &registry_certs,
            emit_bindings: true,
            generation: 0,
            binding_epochs: &BTreeMap::new(),
        },
        &journal,
        deployment_id,
    )
    .await
    .unwrap();
    // `targets` holds a clone of each `Arc<SyneroymClient>` (via
    // `deploy::build_actor`) -- drop it now, not used again, so
    // `shutdown_clients` below sees each client at its sole `Arc` and can
    // actually close its endpoint instead of silently skipping it.
    drop(targets);
    assert!(report.is_complete(), "{:?}", report.failures);

    let backend_svc = new_plan
        .services
        .iter()
        .find(|s| s.logical_ref.service_name.as_str() == "backend")
        .unwrap();

    let registry_client = RegistryClient::new(false, Some(node_a.registry_url.clone()));
    let resolved = resolve_iroh_addr(&registry_client, backend_svc.service_id.as_str())
        .await
        .expect("resolve_iroh_addr for backend's master DID must succeed");
    let node_b_addr = resolve_iroh_addr(&registry_client, node_b.did())
        .await
        .expect("resolve_iroh_addr for node B's own DID must succeed");
    assert_eq!(
        resolved.id, node_b_addr.id,
        "backend's master DID must resolve to node B's own address, not node A's -- this is what \
         §0.1's substrate_id bug would break silently"
    );

    shutdown_clients(clients.into_values()).await;
    node_a.teardown().await;
    node_b.teardown().await;
}

#[tokio::test]
async fn a_certificate_minted_against_one_substrate_is_rejected_by_another() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b, clients) =
        boot_pair(&owner, &operator, PORTS_CERT_REJECTED_BY_ANOTHER_SUBSTRATE).await;

    let master = Identity::generate().unwrap();
    let master_did = substrate::derive_did_key(&master.public_key());

    let registry_client = RegistryClient::new(false, Some(node_a.registry_url.clone()));
    registry_client
        .publish_master_anchor(&master_did, vec![], None, &master, true)
        .await
        .expect("failed to publish member master anchor");

    let client_a = &clients[&SubstrateAlias::new(FRONTEND_ALIAS)];
    let client_b = &clients[&SubstrateAlias::new(BACKEND_ALIAS)];

    // Mint against node A ...
    let cert = certify_instance(client_a, &master, &master_did, 24).await.unwrap();

    // ... deploy to node B: rejected, because the substrate derives the
    // instance key from its own node identity *and* the calling DID, so a
    // certificate minted through node A's client can never match what node B
    // would derive.
    let record = EndpointInfo {
        service_id: master_did.clone(),
        substrate_id: client_b.service_id().to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname: None,
        is_private: false,
        ttl: None,
        not_after: far_future_not_after(),
        generation: 0,
    }
    .sign(&master)
    .unwrap();

    let manifest = DeployManifest {
        config: WitServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
        },
        service_type: WitServiceType::Tcp(TcpManifest {
            endpoints: vec![NetworkEndpoint {
                interface_name: "default".to_string(),
                host: "127.0.0.1".to_string(),
                port: 41303,
            }],
        }),
        registry_certificate: Some(serde_json::to_string(&record).unwrap()),
        instance_certificate: Some(cert.to_json().unwrap()),
    };
    let params = serde_json::to_value((master_did.clone(), manifest)).unwrap();
    let err = client_b.request("orchestrator", "deploy", params).await.unwrap_err();
    assert!(err.to_string().contains("not the key this substrate would derive"), "{err}");

    shutdown_clients(clients.into_values()).await;
    node_a.teardown().await;
    node_b.teardown().await;
}

#[tokio::test]
async fn an_unreachable_substrate_leaves_the_deployment_degraded_and_retryable() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b, clients) =
        boot_pair(&owner, &operator, PORTS_UNREACHABLE_DEGRADED_AND_RETRYABLE).await;

    let plan = compiled_plan().await;
    let (new_plan, masters) = mint_and_substitute_masters(&plan);
    let (instance_certs, registry_certs) = certify_and_publish(&new_plan, &masters, &clients).await;
    let targets = deploy_targets(&clients);

    let journal = DeploymentJournal::open_in_memory().unwrap();
    let instance_id = new_plan.app_instance_id.clone();
    let deployment_id = journal.append(&new_plan, DeploymentState::Applying).unwrap();

    // Stop node B before applying: its own target's client is still built
    // (a real, live `SyneroymClient`), but every call to it now fails.
    let node_b_base_path = node_b.stop_and_keep_dir().await;

    let report = apply_plan(
        ApplyRequest {
            plan: &new_plan,
            targets: &targets,
            fallback: None,
            instance_certificates: &instance_certs,
            registry_certificates: &registry_certs,
            emit_bindings: true,
            generation: 0,
            binding_epochs: &BTreeMap::new(),
        },
        &journal,
        deployment_id,
    )
    .await
    .unwrap();
    // See the other tests' `drop(targets)`: not used again, and holding it
    // open keeps this attempt's dead-node-B client alive past its sole
    // `Arc`, which would make `shutdown_clients` below silently skip it.
    drop(targets);

    assert_eq!(report.deployed.len(), 1, "{report:?}");
    assert_eq!(report.failures.len(), 1, "{report:?}");
    journal.update_state(deployment_id, DeploymentState::Degraded).unwrap();

    let completed = journal.get_completed_actions(deployment_id).unwrap();
    assert_eq!(completed.len(), 1);
    let all_actions_for_instance =
        journal.get_completed_actions_for_instance(&instance_id).unwrap();
    assert_eq!(all_actions_for_instance.len(), 1);

    // Node B returns (same identity, same ports): apply again, resuming.
    let ports = PORTS_UNREACHABLE_DEGRADED_AND_RETRYABLE;
    let node_b = Node::boot(
        node_b_base_path,
        None,
        ports.node_b_iroh,
        ports.node_b_registry,
        ports.node_b_gateway,
        Some(node_a.registry_url.clone()),
        Some(format!("http://localhost:{}", ports.node_a_iroh)),
        &operator,
    )
    .await;
    // The KEK was already injected before the stop; the restarted process
    // reloads its own persisted state (same on-disk identity/data dir), so
    // no second `inject_kek` call is needed here.

    let operator_did = substrate::derive_did_key(&operator.public_key());
    let client_b_again =
        client_for(&node_b, &operator, app_deploy_grant(&owner, &operator_did, node_b.did())).await;
    let mut clients_again = clients;
    if let Some(dead_client_b) =
        clients_again.insert(SubstrateAlias::new(BACKEND_ALIAS), client_b_again)
    {
        shutdown_clients([dead_client_b]).await;
    }
    let targets_again = deploy_targets(&clients_again);

    let report_again = apply_plan(
        ApplyRequest {
            plan: &new_plan,
            targets: &targets_again,
            fallback: None,
            instance_certificates: &instance_certs,
            registry_certificates: &registry_certs,
            emit_bindings: true,
            generation: 0,
            binding_epochs: &BTreeMap::new(),
        },
        &journal,
        deployment_id,
    )
    .await
    .unwrap();
    drop(targets_again);

    assert_eq!(report_again.deployed.len(), 1, "{report_again:?}");
    assert_eq!(
        report_again.skipped.len(),
        1,
        "frontend already landed on node A and must be skipped, not redeployed: {report_again:?}"
    );
    assert!(report_again.is_complete());
    journal.update_state(deployment_id, DeploymentState::Active).unwrap();

    shutdown_clients(clients_again.into_values()).await;
    node_a.teardown().await;
    node_b.teardown().await;
}

#[tokio::test]
async fn a_dependencys_record_resolves_through_the_dependents_own_registry() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b, clients) =
        boot_pair(&owner, &operator, PORTS_DEPENDENTS_OWN_REGISTRY).await;

    let plan = compiled_plan().await;
    let (new_plan, masters) = mint_and_substitute_masters(&plan);
    let (instance_certs, registry_certs) = certify_and_publish(&new_plan, &masters, &clients).await;
    let targets = deploy_targets(&clients);

    let journal = DeploymentJournal::open_in_memory().unwrap();
    let deployment_id = journal.append(&new_plan, DeploymentState::Applying).unwrap();
    let report = apply_plan(
        ApplyRequest {
            plan: &new_plan,
            targets: &targets,
            fallback: None,
            instance_certificates: &instance_certs,
            registry_certificates: &registry_certs,
            emit_bindings: true,
            generation: 0,
            binding_epochs: &BTreeMap::new(),
        },
        &journal,
        deployment_id,
    )
    .await
    .unwrap();
    // `targets` holds a clone of each `Arc<SyneroymClient>` (via
    // `deploy::build_actor`) -- drop it now, not used again, so
    // `shutdown_clients` below sees each client at its sole `Arc` and can
    // actually close its endpoint instead of silently skipping it.
    drop(targets);
    assert!(report.is_complete(), "{:?}", report.failures);

    let backend_svc = new_plan
        .services
        .iter()
        .find(|s| s.logical_ref.service_name.as_str() == "backend")
        .unwrap();

    // `frontend` (node A) resolves `backend` (node B) through node A's own
    // configured registry -- the D-A3-17/§0.12 precondition proven directly:
    // every substrate in play must share one registry namespace (or the
    // DHT), since a substrate publishes only through its own configured
    // registry and nothing on the wire reports which one that is.
    let registry_client_via_a = RegistryClient::new(false, Some(node_a.registry_url.clone()));
    let looked_up = registry_client_via_a
        .lookup(backend_svc.service_id.as_str(), false)
        .await
        .expect("backend's record must resolve through node A's own registry");
    assert_eq!(
        looked_up.info.substrate_id,
        node_b.did(),
        "the record found through node A's registry must name node B as the hosting substrate"
    );

    shutdown_clients(clients.into_values()).await;
    node_a.teardown().await;
    node_b.teardown().await;
}
