#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The epoch-guarded binding write (ADR-0021), proven across two genuinely
//! independent `syneroym-substrate` instances -- a `frontend`-on-A /
//! `backend`-on-B topology, deployed once, then pushed to without a second
//! deploy.
//!
//! Both nodes come from `common::SubstrateNode`, sharing one registry and
//! one relay. `common::serial_guard` keeps this binary's tests from running
//! substrate stacks at once. `shutdown_clients` comes from
//! `common/client_shutdown.rs` because this file holds a batch of
//! `Arc<SyneroymClient>` keyed by alias rather than one client per node.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use common::SubstrateNode;
use rustls::crypto::ring;
use semver::Version;
use serde_json::Map;
use syneroym_app_orchestration::{
    DeploymentJournal, DeploymentPlan, DeploymentState, LocalFilesystemCatalog, Visibility,
    compile,
    models::{
        AppBlueprintId, AppInstanceId, LogicalServiceName, PlacementSelector, ServiceConfig,
        ServiceId, ServiceSpec, ServiceType, SubstrateAlias, SynAppManifest,
    },
};
use syneroym_core::dht_registry::{DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointType};
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};
use syneroym_sdk::{
    BindingWrite, BindingWriteOutcome, DependencyBinding, SyneroymClient, TopologyMode,
    deploy::{self, ApplyRequest, DeployTarget, apply_plan, certify_instance},
};

mod common;

#[path = "common/client_shutdown.rs"]
mod client_shutdown;
use client_shutdown::shutdown_clients;

const FRONTEND_ALIAS: &str = "edge-a";
const BACKEND_ALIAS: &str = "edge-b";

/// An app-scoped `orchestrator/{deploy,undeploy,status}` grant, issued by
/// `node_owner`, letting `grantee_did` deploy/undeploy/list any app on
/// `node_did`.
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
                assets: None,
                visibility: Visibility::Internal,
            },
            depends_on: vec![],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(BACKEND_ALIAS))),
            replicas: 1,
            sharding_strategy: None,
            schedule: None,
            topology_visibility: Default::default(),
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
                assets: None,
                visibility: Visibility::Internal,
            },
            depends_on: vec![LogicalServiceName::new("backend")],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(FRONTEND_ALIAS))),
            replicas: 1,
            sharding_strategy: None,
            schedule: None,
            topology_visibility: Default::default(),
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
    node: &SubstrateNode,
    operator: &Identity,
    grant: CapabilityToken,
) -> Arc<SyneroymClient> {
    let mut client = node.client_as(Identity::from_bytes(&operator.to_bytes())).with_ucan(grant);
    client.connect().await.expect("failed to connect client");
    Arc::new(client)
}

/// Boots both nodes (B sharing A's registry and relay), injects a KEK on
/// each during its own boot (native-capability endpoints need one), and
/// grants `operator` an app-scoped deploy grant on each.
///
/// `owner` and `operator` must be **distinct** identities: `owner` becomes
/// each node's `admin_ucan_root`, so if `operator` were the same identity
/// its app-scoped grant would be moot.
async fn boot_pair(
    owner: &Identity,
    operator: &Identity,
) -> (SubstrateNode, SubstrateNode, BTreeMap<SubstrateAlias, Arc<SyneroymClient>>) {
    let _ = ring::default_provider().install_default();

    // Each node injects its own KEK during its own boot, before the sibling's
    // boot can leave its connection idle -- so no post-boot redial is needed.
    let node_a = SubstrateNode::builder().owner(owner).inject_kek_bytes([0xaa; 32]).boot().await;
    let node_b = SubstrateNode::builder()
        .owner(owner)
        .shared_registry(node_a.registry_url())
        .shared_relay(node_a.relay_url())
        .inject_kek_bytes([0xbb; 32])
        .boot()
        .await;

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

        if svc.config.visibility == Visibility::Private {
            continue;
        }

        let record = EndpointInfo {
            service_id: svc.service_id.to_string(),
            substrate_id: client.service_id().to_string(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: None,
            is_private: svc.config.visibility == Visibility::Internal,
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
            binding_epochs: &BTreeMap::new(),
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
    let _serial_guard = common::serial_guard().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b, clients) = boot_pair(&owner, &operator).await;

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

    shutdown_clients(clients.into_values()).await;
    node_a.teardown().await;
    node_b.teardown().await;
}

/// Matrix row 5, live: a late-arriving retry presenting an epoch below the
/// one already held must not regress the mapping, proven against a real
/// substrate rather than the pure `classify_binding_write` unit test.
#[tokio::test]
async fn a_stale_epoch_push_does_not_regress_the_mapping() {
    let _serial_guard = common::serial_guard().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b, clients) = boot_pair(&owner, &operator).await;

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

    shutdown_clients(clients.into_values()).await;
    node_a.teardown().await;
    node_b.teardown().await;
}
