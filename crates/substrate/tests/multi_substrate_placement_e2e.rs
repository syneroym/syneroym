#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Multi-substrate placement and the substrate inventory, proven across two
//! genuinely independent `syneroym-substrate` instances -- the reference
//! scenario's `frontend`-on-A/`backend`-on-B topology, driven through the
//! same `crates/sdk::deploy` executor `roymctl app deploy` uses (`sdk` is
//! only a dev-dependency of this crate, and `roymctl` is a binary that
//! cannot be linked from a test -- so this harness calls `compile`,
//! `certify_placed_members`, and `apply_plan` directly, mirroring what
//! `roymctl`'s own per-substrate member-identity substitution does).
//!
//! Both nodes come from `common::SubstrateNode`, sharing one registry and
//! one relay. `common::serial_guard` keeps this binary's tests from running
//! substrate stacks at once.

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
use syneroym_core::dht_registry::{
    DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointType, RegistryClient,
};
use syneroym_identity::{Identity, substrate};
use syneroym_router::net_iroh::resolve_iroh_addr;
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};
use syneroym_sdk::{
    DeployManifest, NetworkEndpoint, ServiceConfig as WitServiceConfig,
    ServiceType as WitServiceType, SyneroymClient, TcpManifest, Visibility as WitVisibility,
    deploy::{
        self, ApplyRequest, DeployTarget, apply_plan, certify_instance, member_registry_record,
    },
};

mod common;

#[path = "common/client_shutdown.rs"]
mod client_shutdown;
use client_shutdown::shutdown_clients;

const FRONTEND_ALIAS: &str = "edge-a";
const BACKEND_ALIAS: &str = "edge-b";

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
        id: AppBlueprintId::new("syneroym:a3-test-app"),
        version: Version::new(0, 1, 0),
        description: None,
        placement: None,
        services,
        dependencies: BTreeMap::new(),
    }
}

/// Two independent (no `depends_on`) services, both placed on
/// `BACKEND_ALIAS` and both declaring no visibility (`Visibility::Private`
/// by default) -- test 37's shape. Same alias, deliberately: a
/// cross-substrate `depends_on` onto a `private` member is refused by
/// `validate_plan_visibility` (`D-B2-14`(a)) before a deploy is even
/// attempted, which would prove the compiler's refusal rather than F1's
/// "undeclared = unpublished" consequence this test is actually about.
fn two_service_manifest_same_alias_private() -> SynAppManifest {
    let mut services = BTreeMap::new();
    services.insert(
        LogicalServiceName::new("backend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41304".to_string(),
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
                visibility: Visibility::Private,
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
        LogicalServiceName::new("sibling"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41305".to_string(),
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
                visibility: Visibility::Private,
            },
            depends_on: vec![],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(BACKEND_ALIAS))),
            replicas: 1,
            sharding_strategy: None,
            schedule: None,
            topology_visibility: Default::default(),
        },
    );
    SynAppManifest {
        id: AppBlueprintId::new("syneroym:a3-undeclared-app"),
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
    node: &SubstrateNode,
    operator: &Identity,
    grant: CapabilityToken,
) -> Arc<SyneroymClient> {
    let mut client = node.client_as(Identity::from_bytes(&operator.to_bytes())).with_ucan(grant);
    client.connect().await.expect("failed to connect client");
    Arc::new(client)
}

/// Boots both nodes (B sharing A's registry and relay), grants `operator` an
/// app-scoped deploy grant on each, injects a KEK on each (native-capability
/// endpoints need one), and returns everything a test needs to drive a
/// deploy. When `node_b_base_path` is `Some`, node B boots from that
/// caller-owned directory so it can be torn down and rebooted under the same
/// on-disk identity later in the same test.
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
    node_b_base_path: Option<PathBuf>,
) -> (SubstrateNode, SubstrateNode, BTreeMap<SubstrateAlias, Arc<SyneroymClient>>) {
    let _ = ring::default_provider().install_default();

    // Each node injects its own KEK during its own boot, before the sibling's
    // boot can leave its connection idle -- so no post-boot redial is needed.
    let node_a = SubstrateNode::builder().owner(owner).inject_kek_bytes([0xaa; 32]).boot().await;
    let mut node_b_builder = SubstrateNode::builder()
        .owner(owner)
        .shared_registry(node_a.registry_url())
        .shared_relay(node_a.relay_url())
        .inject_kek_bytes([0xbb; 32]);
    if let Some(path) = node_b_base_path {
        node_b_builder = node_b_builder.base_path(path);
    }
    let node_b = node_b_builder.boot().await;

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
/// concrete clients above -- this fixture always places every service
/// explicitly (no `fallback` client) and wants a far-future `not_after`
/// instead of the production default, which is why it is not simply a call
/// to `certify_placed_members` itself. The registry-record half calls the
/// exact same [`member_registry_record`] the production function does, so
/// the visibility -> record decision (`D-B2-7`) cannot drift between this
/// harness and production; that decision's own direct coverage lives in
/// `crates/sdk`'s unit tests, since it needs no network and this harness
/// only needs its result.
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

        if let Some(record_json) = member_registry_record(
            svc.config.visibility,
            svc.service_id.as_str(),
            client.service_id(),
            master,
            far_future_not_after(),
        )
        .unwrap()
        {
            records.insert(svc.service_id.clone(), record_json);
        }
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
    let _serial_guard = common::serial_guard().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b, clients) = boot_pair(&owner, &operator, None).await;

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
    let _serial_guard = common::serial_guard().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b, clients) = boot_pair(&owner, &operator, None).await;

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

    let registry_client = RegistryClient::new(false, Some(node_a.registry_url().to_string()));
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

/// Test 37: an app deployed with no visibility declaration publishes no
/// member records, and a cross-node dial for one of its members fails to
/// resolve -- `D-B2-3`'s consequence made real on the app path, not just
/// asserted at the unit level (F1: `certify_placed_members` used to mint a
/// record for every placed member unconditionally).
#[tokio::test]
async fn an_app_deployed_with_no_visibility_declaration_publishes_no_member_records() {
    let _serial_guard = common::serial_guard().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b, clients) = boot_pair(&owner, &operator, None).await;

    let manifest = two_service_manifest_same_alias_private();
    let catalog = LocalFilesystemCatalog::new(std::path::PathBuf::from("."));
    let compiled =
        compile(AppInstanceId::new("a3-undeclared-inst"), &manifest, &catalog).await.unwrap();
    let plan = compiled.plans.last().unwrap().clone();
    let (new_plan, masters) = mint_and_substitute_masters(&plan);
    // Mirrors `certify_placed_members`'s own behaviour (F1/D-B2-7): a
    // `private` member gets no entry in the returned map at all.
    let (instance_certs, registry_certs) = certify_and_publish(&new_plan, &masters, &clients).await;
    assert!(
        registry_certs.is_empty(),
        "no member declares a visibility, so none should have a minted registry certificate: \
         {registry_certs:?}"
    );
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
    drop(targets);
    assert!(report.is_complete(), "{:?}", report.failures);

    let backend_svc = new_plan
        .services
        .iter()
        .find(|s| s.logical_ref.service_name.as_str() == "backend")
        .unwrap();

    let registry_client = RegistryClient::new(false, Some(node_a.registry_url().to_string()));
    let err =
        resolve_iroh_addr(&registry_client, backend_svc.service_id.as_str()).await.expect_err(
            "a member with no visibility declaration must never resolve to an address across \
             installations -- absence must not mean 'publish it'",
        );
    let _ = err;

    shutdown_clients(clients.into_values()).await;
    node_a.teardown().await;
    node_b.teardown().await;
}

#[tokio::test]
async fn a_certificate_minted_against_one_substrate_is_rejected_by_another() {
    let _serial_guard = common::serial_guard().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b, clients) = boot_pair(&owner, &operator, None).await;

    let master = Identity::generate().unwrap();
    let master_did = substrate::derive_did_key(&master.public_key());

    let registry_client = RegistryClient::new(false, Some(node_a.registry_url().to_string()));
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
        // Matches the manifest's `visibility: Internal` below. Unreached in
        // practice -- the instance-certificate mismatch this test is about
        // is checked first -- but kept consistent so a reader does not
        // mistake it for a second, unrelated bug.
        is_private: true,
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
            assets: None,
            visibility: Some(WitVisibility::Internal),
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
    let _serial_guard = common::serial_guard().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    // `node_b_dir` is a `TempDir` this test owns directly so it survives
    // `node_b`'s own teardown below -- the whole point is to reboot the
    // *same* identity, not a fresh one.
    let node_b_dir = tempfile::tempdir().expect("failed to create temp dir");
    let (node_a, node_b, clients) =
        boot_pair(&owner, &operator, Some(node_b_dir.path().to_path_buf())).await;
    let shared_registry = node_a.registry_url().to_string();
    let shared_relay = node_a.relay_url().to_string();

    let plan = compiled_plan().await;
    let (new_plan, masters) = mint_and_substitute_masters(&plan);
    let (instance_certs, registry_certs) = certify_and_publish(&new_plan, &masters, &clients).await;
    let targets = deploy_targets(&clients);

    let journal = DeploymentJournal::open_in_memory().unwrap();
    let instance_id = new_plan.app_instance_id.clone();
    let deployment_id = journal.append(&new_plan, DeploymentState::Applying).unwrap();

    // Stop node B before applying: its own target's client is still built
    // (a real, live `SyneroymClient`), but every call to it now fails. Its
    // on-disk identity and data survive in `node_b_dir`.
    node_b.teardown().await;

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

    // Node B returns, same identity (same `base_path`). Fresh ports are
    // fine: node B's client resolves its DID through node A's still-live
    // registry, and the rebooted node re-publishes its new address.
    let node_b = SubstrateNode::builder()
        .owner(&operator)
        .base_path(node_b_dir.path())
        .shared_registry(&shared_registry)
        .shared_relay(&shared_relay)
        .boot()
        .await;
    // The KEK was already injected before the stop; the restarted process
    // reloads its own persisted state (same on-disk identity and data dir),
    // so no second `inject_kek` call is needed here.

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
    let _serial_guard = common::serial_guard().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b, clients) = boot_pair(&owner, &operator, None).await;

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
    let registry_client_via_a = RegistryClient::new(false, Some(node_a.registry_url().to_string()));
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
