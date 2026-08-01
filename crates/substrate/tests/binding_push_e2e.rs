#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The epoch-guarded binding write (M05A Slice A5a, ADR-0021 §3), proven
//! across two genuinely independent `syneroym-substrate` instances -- the
//! same `frontend`-on-A/`backend`-on-B topology `multi_substrate_placement_
//! e2e.rs` uses, deployed once, then pushed to without a second deploy.
//!
//! `Node` and the deploy scaffolding are copied from `multi_substrate_
//! placement_e2e.rs`, which itself copies them from `master_endpoint_record_
//! e2e.rs` -- this crate has no shared test-support module, so every e2e
//! file duplicates the harness it needs (see that file's own module doc).

use std::{
    collections::BTreeMap,
    path::PathBuf,
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
    dht_registry::{DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointType},
};
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};
use syneroym_sdk::{
    BindingWrite, BindingWriteOutcome, DependencyBinding, SyneroymClient, TopologyMode,
    deploy::{ApplyRequest, DeployTarget, SubstrateActor, apply_plan, certify_instance},
};
use syneroym_substrate::identity;
use tempfile::TempDir;
use tokio::{
    sync::{mpsc, mpsc::Sender},
    task::JoinHandle,
};

/// Each `#[tokio::test]` in this file runs concurrently, so each of the two
/// tests needs its own, non-overlapping port block -- the same convention
/// `multi_substrate_placement_e2e.rs` follows.
#[derive(Clone, Copy)]
struct PortBlock {
    node_a_iroh: u16,
    node_a_registry: u16,
    node_a_gateway: u16,
    node_b_iroh: u16,
    node_b_registry: u16,
    node_b_gateway: u16,
}

const PORTS_MEMBERSHIP_CHANGE_TAKES_EFFECT: PortBlock = PortBlock {
    node_a_iroh: 10600,
    node_a_registry: 10601,
    node_a_gateway: 10602,
    node_b_iroh: 10700,
    node_b_registry: 10701,
    node_b_gateway: 10702,
};
const PORTS_STALE_PUSH_DOES_NOT_REGRESS: PortBlock = PortBlock {
    node_a_iroh: 10800,
    node_a_registry: 10801,
    node_a_gateway: 10802,
    node_b_iroh: 10900,
    node_b_registry: 10901,
    node_b_gateway: 10902,
};

const FRONTEND_ALIAS: &str = "edge-a";
const BACKEND_ALIAS: &str = "edge-b";

/// A full, independently-identified `syneroym-substrate` instance. Copied
/// from `multi_substrate_placement_e2e.rs`'s own `Node`.
struct Node {
    registry_url: String,
    substrate_client: SyneroymClient,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
    _temp_dir: TempDir,
}

impl Node {
    /// `owner`'s DID becomes this node's `[iam].admin_ucan_root` (an unowned
    /// substrate now fails closed).
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
            registry_url: effective_registry_url,
            substrate_client,
            shutdown_tx,
            substrate_handle,
            _temp_dir: temp_dir,
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

/// An app-scoped `orchestrator/{deploy,undeploy,status}` grant, issued by
/// `node_owner`, letting `grantee_did` deploy/undeploy/list any app on
/// `node_did`. Copied from `multi_substrate_placement_e2e.rs`.
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
/// `edge-b`) -- the reference scenario's own topology, copied from
/// `multi_substrate_placement_e2e.rs`.
fn two_service_manifest() -> SynAppManifest {
    let mut services = BTreeMap::new();
    services.insert(
        LogicalServiceName::new("backend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41401".to_string(),
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
        },
    );
    services.insert(
        LogicalServiceName::new("frontend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41402".to_string(),
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
        },
    );
    SynAppManifest {
        id: AppBlueprintId::new("syneroym:a5a-binding-push-test-app"),
        version: Version::new(0, 1, 0),
        description: None,
        placement: None,
        services,
        dependencies: BTreeMap::new(),
    }
}

/// Mints one member master per planned service and substitutes each
/// `service_id`/`resolved_dependencies` entry with the resolved master DID.
/// Copied from `multi_substrate_placement_e2e.rs`.
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

/// Boots both nodes (B sharing A's registry/relay), grants `operator` an
/// app-scoped deploy grant on each, and injects a KEK on each. Copied from
/// `multi_substrate_placement_e2e.rs`'s own `boot_pair`.
async fn boot_pair(
    owner: &Identity,
    operator: &Identity,
    ports: PortBlock,
) -> (Node, Node, BTreeMap<SubstrateAlias, Arc<SyneroymClient>>) {
    let _ = ring::default_provider().install_default();

    let node_a = Node::boot_fresh(
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

    node_a.substrate_client.inject_kek("aa".repeat(32)).await.expect("node A inject_kek failed");
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

/// Certifies and signs an endpoint record for each master. Copied from
/// `multi_substrate_placement_e2e.rs`.
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
                    actor: c.clone() as Arc<dyn SubstrateActor>,
                },
            )
        })
        .collect()
}

async fn compiled_plan() -> DeploymentPlan {
    let manifest = two_service_manifest();
    let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
    let compiled =
        compile(AppInstanceId::new("a5a-binding-push-inst"), &manifest, &catalog).await.unwrap();
    compiled.plans.last().unwrap().clone()
}

/// Deploys the reference-scenario topology across both nodes and returns
/// the master-substituted plan alongside the two connected clients, ready
/// for a test to push a binding change against.
async fn deploy_two_service_app(
    clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
) -> DeploymentPlan {
    let plan = compiled_plan().await;
    let (new_plan, masters) = mint_and_substitute_masters(&plan);
    let (instance_certs, registry_certs) = certify_and_publish(&new_plan, &masters, clients).await;
    let targets = deploy_targets(clients);

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
        },
        &journal,
        deployment_id,
    )
    .await
    .unwrap();
    assert!(report.is_complete(), "{:?}", report.failures);
    new_plan
}

/// The core claim ADR-0021 §1/§3 exist for: a membership change reaches a
/// dependent deployed on a *different* node without a redeploy. Verified
/// through `status`'s `binding-epochs` (M05A A5a §6) -- the persisted,
/// per-dependent row `write-bindings` updates -- rather than a live guest
/// call: no wasm test-component in this tree exports a `dependency(...)`
/// test-driver interface, and building one is a larger undertaking than
/// this test's own scope. "Without a redeploy" is structural here: the
/// test issues exactly one `deploy` call and one `write-bindings` call,
/// never a second deploy.
#[tokio::test]
async fn a_membership_change_pushed_to_a_dependent_takes_effect_without_a_redeploy() {
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b, clients) =
        boot_pair(&owner, &operator, PORTS_MEMBERSHIP_CHANGE_TAKES_EFFECT).await;

    let plan = deploy_two_service_app(&clients).await;
    let client_a = &clients[&SubstrateAlias::new(FRONTEND_ALIAS)];
    let frontend_svc =
        plan.services.iter().find(|s| s.logical_ref.service_name.as_str() == "frontend").unwrap();

    // The initial deploy emits the binding at epoch 0 (A2 mints no
    // epochs; the supervisor does).
    let before = client_a.status(vec![frontend_svc.service_id.to_string()]).await.unwrap();
    assert_eq!(before.services.len(), 1, "{before:?}");
    assert_eq!(before.services[0].binding_epochs, vec![("backend".to_string(), 0)]);

    let new_backend_member =
        ServiceId::new(substrate::derive_did_key(&Identity::generate().unwrap().public_key()));
    let outcomes = client_a
        .write_bindings(BindingWrite {
            service_id: frontend_svc.service_id.to_string(),
            app_instance_id: plan.app_instance_id.to_string(),
            bindings: vec![DependencyBinding {
                dependency_name: "backend".to_string(),
                app_instance_id: plan.app_instance_id.to_string(),
                mode: TopologyMode::Singleton,
                members: vec![new_backend_member.to_string()],
                epoch: 1,
                cache_ttl_ms: 60_000,
            }],
            generation: 0,
        })
        .await
        .unwrap();
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(outcomes[0], BindingWriteOutcome::Applied), "{outcomes:?}");

    let after = client_a.status(vec![frontend_svc.service_id.to_string()]).await.unwrap();
    assert_eq!(
        after.services[0].binding_epochs,
        vec![("backend".to_string(), 1)],
        "the pushed epoch must be visible without a second deploy"
    );

    node_a.teardown().await;
    node_b.teardown().await;
}

/// Matrix row 5, live: a late-arriving retry presenting an epoch below the
/// one already held must not regress the mapping, proven against a real
/// substrate rather than the pure `classify_binding_write` unit test.
#[tokio::test]
async fn a_stale_epoch_push_does_not_regress_the_mapping() {
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b, clients) =
        boot_pair(&owner, &operator, PORTS_STALE_PUSH_DOES_NOT_REGRESS).await;

    let plan = deploy_two_service_app(&clients).await;
    let client_a = &clients[&SubstrateAlias::new(FRONTEND_ALIAS)];
    let frontend_svc =
        plan.services.iter().find(|s| s.logical_ref.service_name.as_str() == "frontend").unwrap();

    let current_member =
        ServiceId::new(substrate::derive_did_key(&Identity::generate().unwrap().public_key()));
    client_a
        .write_bindings(BindingWrite {
            service_id: frontend_svc.service_id.to_string(),
            app_instance_id: plan.app_instance_id.to_string(),
            bindings: vec![DependencyBinding {
                dependency_name: "backend".to_string(),
                app_instance_id: plan.app_instance_id.to_string(),
                mode: TopologyMode::Singleton,
                members: vec![current_member.to_string()],
                epoch: 2,
                cache_ttl_ms: 60_000,
            }],
            generation: 0,
        })
        .await
        .unwrap();

    // A late-arriving retry of an older write: a lower epoch, a different
    // membership than what is now held.
    let stale_member =
        ServiceId::new(substrate::derive_did_key(&Identity::generate().unwrap().public_key()));
    let outcomes = client_a
        .write_bindings(BindingWrite {
            service_id: frontend_svc.service_id.to_string(),
            app_instance_id: plan.app_instance_id.to_string(),
            bindings: vec![DependencyBinding {
                dependency_name: "backend".to_string(),
                app_instance_id: plan.app_instance_id.to_string(),
                mode: TopologyMode::Singleton,
                members: vec![stale_member.to_string()],
                epoch: 1,
                cache_ttl_ms: 60_000,
            }],
            generation: 0,
        })
        .await
        .unwrap();
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(outcomes[0], BindingWriteOutcome::Stale(2)), "{outcomes:?}");

    let status = client_a.status(vec![frontend_svc.service_id.to_string()]).await.unwrap();
    assert_eq!(
        status.services[0].binding_epochs,
        vec![("backend".to_string(), 2)],
        "the stale push must not regress the mapping below the epoch already held"
    );

    node_a.teardown().await;
    node_b.teardown().await;
}
