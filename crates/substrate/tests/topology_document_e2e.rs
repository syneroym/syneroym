#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Tier 2 of the logical discovery overlay (ADR-0022 §3), proven across two
//! genuinely independent `syneroym-substrate` instances -- the milestone's
//! reference scenario, steps 3 through 8: a caller outside the app instance
//! fetches the signed topology document, verifies it, routes from it, and
//! keeps routing after the supervisor that signed it goes away.
//!
//! `Node`, `boot_pair`, `supervisor_role`, `compiled_plan_json`,
//! `node_wide_supervisor_grant`, and `submission` are copied from
//! `tier1_endpoint_record_e2e.rs`, which itself copied them from
//! `app_instance_identity_e2e.rs`.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use rustls::crypto::ring;
use semver::Version;
use serde_json::{Map, json};
use syneroym_app_orchestration::{
    AppDid, LocalFilesystemCatalog, LogicalResolver, LogicalServiceName, SignedTopologyDocument,
    StaticInventory, TopologyFetcher, compile,
    models::{
        AppBlueprintId, AppInstanceId, PlacementSelector, ServiceConfig, ServiceSpec, ServiceType,
        SubstrateAlias, SynAppManifest,
    },
    register_verified,
};
use syneroym_app_supervisor::inventory::SupervisorInventoryEntry;
use syneroym_core::{
    config::{
        ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole, IrohParentConfig, LogTarget,
        ServiceRegistryRole, SubstrateConfig, SupervisorRole,
    },
    dht_registry::RegistryClient,
};
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};
use syneroym_sdk::{RegistryTopologyFetcher, SyneroymClient, fetch_and_register};
use syneroym_substrate::identity;
use tempfile::TempDir;
use tokio::{
    sync::{mpsc, mpsc::Sender},
    task::JoinHandle,
    time,
};

#[path = "common/retry.rs"]
mod retry;

#[derive(Clone, Copy)]
struct PortBlock {
    supervisor_iroh: u16,
    supervisor_registry: u16,
    supervisor_gateway: u16,
    managed_iroh: u16,
    managed_registry: u16,
    managed_gateway: u16,
}

// The next free block after `tier1_endpoint_record_e2e.rs`'s 15_000-15_302,
// the highest claimed at the time this file was written -- one block of six
// per test, since cargo runs every test in this binary concurrently.
const PORTS_OUTSIDE_CALLER_RESOLVES: PortBlock = PortBlock {
    supervisor_iroh: 15_400,
    supervisor_registry: 15_401,
    supervisor_gateway: 15_402,
    managed_iroh: 15_500,
    managed_registry: 15_501,
    managed_gateway: 15_502,
};
const PORTS_RELAYED_DOCUMENT_VERIFIES: PortBlock = PortBlock {
    supervisor_iroh: 15_600,
    supervisor_registry: 15_601,
    supervisor_gateway: 15_602,
    managed_iroh: 15_700,
    managed_registry: 15_701,
    managed_gateway: 15_702,
};
const PORTS_SCALE_OUT_SUPERSEDES: PortBlock = PortBlock {
    supervisor_iroh: 15_800,
    supervisor_registry: 15_801,
    supervisor_gateway: 15_802,
    managed_iroh: 15_900,
    managed_registry: 15_901,
    managed_gateway: 15_902,
};
const PORTS_CACHED_DOCUMENT_SURVIVES_SUPERVISOR_DOWN: PortBlock = PortBlock {
    supervisor_iroh: 16_000,
    supervisor_registry: 16_001,
    supervisor_gateway: 16_002,
    managed_iroh: 16_100,
    managed_registry: 16_101,
    managed_gateway: 16_102,
};
const PORTS_NO_CACHE_FAILS_CLEANLY: PortBlock = PortBlock {
    supervisor_iroh: 16_200,
    supervisor_registry: 16_201,
    supervisor_gateway: 16_202,
    managed_iroh: 16_300,
    managed_registry: 16_301,
    managed_gateway: 16_302,
};
const PORTS_FORGED_DOCUMENT_REJECTED: PortBlock = PortBlock {
    supervisor_iroh: 16_400,
    supervisor_registry: 16_401,
    supervisor_gateway: 16_402,
    managed_iroh: 16_500,
    managed_registry: 16_501,
    managed_gateway: 16_502,
};

const MANAGED_ALIAS: &str = "managed";

/// A full, independently-identified `syneroym-substrate` instance.
struct Node {
    registry_url: String,
    substrate_client: SyneroymClient,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
    _temp_dir: TempDir,
}

impl Node {
    #[allow(clippy::too_many_arguments)]
    async fn boot(
        iroh_port: u16,
        registry_port: u16,
        gateway_port: u16,
        shared_registry_url: Option<String>,
        shared_relay_url: Option<String>,
        owner: &Identity,
        supervisor: Option<SupervisorRole>,
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
        config.substrate.enable_bep0044_dht = false;
        let relay_url = shared_relay_url.unwrap_or_else(|| format!("http://localhost:{iroh_port}"));
        config.parent_coordinator.iroh = Some(IrohParentConfig { url: relay_url });
        config.roles.client_gateway =
            Some(ClientGatewayRole { http_port: gateway_port, ..Default::default() });
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

/// `poll_interval_secs` is lowered from the 30s default so this file's
/// tests do not have to wait a full poll cycle for anything that depends
/// on the resident loop (none of `resolve`'s own tests do -- it is a
/// direct RPC, not loop-triggered -- but `boot_pair`'s shared shape keeps
/// this for consistency with `tier1_endpoint_record_e2e.rs`).
fn supervisor_role() -> SupervisorRole {
    SupervisorRole {
        poll_interval_secs: 2,
        db_name: "supervisor.db".to_string(),
        max_restart_attempts: 3,
        restart_backoff_secs: 30,
        alert_topic: "supervisor/alerts".to_string(),
        master_backup_dir: "master-backups".to_string(),
        ..SupervisorRole::default()
    }
}

/// Node-wide `orchestrator/deploy` **and** `orchestrator/status` for
/// `grantee_did` on `node_did` -- what a supervisor needs on every substrate
/// it manages.
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

/// `supervisor/resolve` on `synapp:<app_did>` (ADR-0022 §5), issued from
/// the supervisor node's own owner -- which `Node::boot` installs as
/// `config.iam.admin_ucan_root` -- to a caller identity that holds no
/// `substrate/admin` and is not part of the app instance. This is what
/// makes the reference scenario's step 3 an honest test: the outside
/// caller genuinely has no other standing on this node.
fn resolve_grant(supervisor_owner: &Identity, grantee_did: &str, app_did: &str) -> CapabilityToken {
    CapabilityToken::issue(
        supervisor_owner,
        grantee_did,
        vec![Capability {
            with: ResourceUri(format!("synapp:{app_did}")),
            can: Ability(Ability::SUPERVISOR_RESOLVE.to_string()),
            caveats: None,
        }],
        Map::new(),
        3600,
        vec![],
    )
    .expect("issue resolve grant")
}

/// `replicas`-member manifest, `backend` placed on `MANAGED_ALIAS`.
fn service_manifest(replicas: u32) -> SynAppManifest {
    let mut services = BTreeMap::new();
    services.insert(
        LogicalServiceName::new("backend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41901".to_string(),
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
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_ALIAS))),
            replicas,
            sharding_strategy: None,
            schedule: None,
        },
    );
    SynAppManifest {
        id: AppBlueprintId::new("syneroym:topology-document-test-app"),
        version: Version::new(0, 1, 0),
        description: None,
        placement: None,
        services,
        dependencies: BTreeMap::new(),
    }
}

async fn compiled_plan_json(manifest: &SynAppManifest, instance_id: &str) -> String {
    let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
    let compiled = compile(AppInstanceId::new(instance_id), manifest, &catalog).await.unwrap();
    compiled.plans.last().unwrap().to_json().unwrap()
}

/// Boots a supervisor node and a managed node (B sharing A's
/// registry/relay), grants the supervisor's own node-wide `orchestrator/
/// deploy` on the managed node, and returns everything a test needs to call
/// `submit`. Copied from `tier1_endpoint_record_e2e.rs`.
async fn boot_pair(
    supervisor_owner: &Identity,
    managed_owner: &Identity,
    ports: PortBlock,
) -> (Node, Node, String) {
    let _ = ring::default_provider().install_default();

    let supervisor_node = Node::boot(
        ports.supervisor_iroh,
        ports.supervisor_registry,
        ports.supervisor_gateway,
        None,
        None,
        supervisor_owner,
        Some(supervisor_role()),
    )
    .await;
    let shared_registry = supervisor_node.registry_url.clone();
    let shared_relay = format!("http://localhost:{}", ports.supervisor_iroh);
    let managed_node = Node::boot(
        ports.managed_iroh,
        ports.managed_registry,
        ports.managed_gateway,
        Some(shared_registry),
        Some(shared_relay),
        managed_owner,
        None,
    )
    .await;

    let grant =
        node_wide_supervisor_grant(managed_owner, supervisor_node.did(), managed_node.did());
    let inventory_json = serde_json::to_string(&BTreeMap::from([(
        MANAGED_ALIAS.to_string(),
        SupervisorInventoryEntry {
            did: managed_node.did().to_string(),
            api_url: Some(managed_node.registry_url.clone()),
            ucan: Some(grant),
        },
    )]))
    .unwrap();

    (supervisor_node, managed_node, inventory_json)
}

fn submission(
    instance_id: &str,
    plan_json: String,
    inventory_json: String,
    generation: u64,
) -> serde_json::Value {
    json!([{
        "app_instance_id": instance_id,
        "plan_json": plan_json,
        "inventory_json": inventory_json,
        "generation": generation,
    }])
}

/// Submits and adopts a `replicas`-member instance, returning the app
/// master DID -- the reference scenario's step 1.
async fn submit_and_adopt(
    supervisor_node: &mut Node,
    instance_id: &str,
    inventory_json: String,
    replicas: u32,
) -> String {
    let manifest = service_manifest(replicas);
    let plan_json = compiled_plan_json(&manifest, instance_id).await;
    // `supervisor_node`'s connection was dialed and proven live by its own
    // `wait_for_ready` during `Node::boot` inside `boot_pair`, then sat
    // idle through the managed node's own full boot that followed -- long
    // enough under CI's scheduling pressure for the peer to abandon that
    // idle path ("no viable network path exists: last path abandoned by
    // peer"; same root cause fixed throughout this crate's e2e tests, e.g.
    // `binding_push_e2e.rs`). Recover by explicit shutdown→reconnect
    // before one retry, not just retrying the same request on the same
    // dead connection.
    let submit_params = submission(instance_id, plan_json, inventory_json, 0);
    crate::call_with_reconnect!(
        supervisor_node.substrate_client,
        "supervisor",
        "submit",
        submit_params
    );
    let adopted = supervisor_node
        .substrate_client
        .request("supervisor", "adopt", json!([instance_id]))
        .await
        .expect("adopt failed");
    let app_did = adopted
        .result
        .get("app_master_did")
        .and_then(|v| v.as_str())
        .expect("adopt-result carries app_master_did")
        .to_string();

    // Tier 1 (this app DID -> its supervising node) is published by the
    // resident loop's own tick, not synchronously by `adopt` -- a `resolve`
    // fetch's own Tier-1 lookup would otherwise race it. Waited out here,
    // once, rather than in every test.
    let registry_client = RegistryClient::new(false, Some(supervisor_node.registry_url.clone()));
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if registry_client.lookup(&app_did, false).await.is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the Tier-1 record for {app_did} never resolved through the registry"
        );
        time::sleep(Duration::from_millis(300)).await;
    }

    app_did
}

/// A fetcher connecting to the supervisor node as `caller`, presenting
/// `grant`.
fn outside_caller_fetcher(
    supervisor_registry_url: &str,
    caller: &Identity,
    grant: CapabilityToken,
) -> RegistryTopologyFetcher {
    RegistryTopologyFetcher::new(supervisor_registry_url.to_string())
        .with_identity(caller)
        .with_ucan(grant)
        .with_connect_timeout(Duration::from_secs(10))
}

/// Reference scenario steps 3 and 5: an outside caller -- not part of the
/// app instance, holding only `supervisor/resolve` -- fetches the Tier-2
/// document and routes to one of its members.
#[tokio::test]
async fn an_outside_caller_resolves_an_apps_members_and_calls_one() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, PORTS_OUTSIDE_CALLER_RESOLVES).await;

    let app_did =
        submit_and_adopt(&mut supervisor_node, "resolve-outside-inst", inventory_json, 2).await;

    let outside_caller = Identity::generate().unwrap();
    let outside_caller_did = substrate::derive_did_key(&outside_caller.public_key());
    let grant = resolve_grant(&supervisor_owner, &outside_caller_did, &app_did);
    let fetcher = outside_caller_fetcher(&supervisor_node.registry_url, &outside_caller, grant);

    let app_did_typed = AppDid::new(app_did.clone());
    let resolver = LogicalResolver::new(Arc::new(StaticInventory::new()));
    let key = fetch_and_register(
        &fetcher,
        &resolver,
        &app_did_typed,
        &LogicalServiceName::new("backend"),
    )
    .await
    .expect("fetch_and_register failed");

    // Step 5: route to a member. Tier 3 (turning that DID into an address)
    // is an ordinary registry lookup, unchanged by this milestone and
    // already proven at unit scale elsewhere -- this asserts the member
    // this overlay resolved to is a real one, not that Tier 3 itself works.
    let member = resolver.resolve(&key, Some(b"routing-key")).expect("resolve failed");
    let all = resolver.resolve_all(&key).unwrap();
    assert_eq!(all.members.len(), 2, "the two-replica backend must resolve both members");
    assert!(all.members.contains(&member));

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

/// Reference scenario step 4, matrix row 5: a document relayed by a party
/// that never contacted the supervisor must verify identically from bytes
/// alone.
#[tokio::test]
async fn a_relayed_document_verifies_for_a_party_that_never_contacted_the_supervisor() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, PORTS_RELAYED_DOCUMENT_VERIFIES).await;

    let app_did =
        submit_and_adopt(&mut supervisor_node, "relayed-doc-inst", inventory_json, 1).await;

    let outside_caller = Identity::generate().unwrap();
    let outside_caller_did = substrate::derive_did_key(&outside_caller.public_key());
    let grant = resolve_grant(&supervisor_owner, &outside_caller_did, &app_did);
    let fetcher = outside_caller_fetcher(&supervisor_node.registry_url, &outside_caller, grant);

    let app_did_typed = AppDid::new(app_did.clone());
    let signed = fetcher
        .fetch(&app_did_typed, &LogicalServiceName::new("backend"))
        .await
        .expect("fetch failed");

    // "Relay": serialize to bytes, drop everything that ever touched the
    // network, and verify from the bytes and the app DID alone.
    let bytes = serde_json::to_vec(&signed).unwrap();
    drop(fetcher);
    drop(outside_caller);

    let relayed: SignedTopologyDocument = serde_json::from_slice(&bytes).unwrap();
    assert!(relayed.verify(&app_did_typed).is_ok());

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

/// Reference scenario step 6: the member set changes and the epoch
/// increments; a cached document is superseded and a re-resolve returns
/// the new set.
#[tokio::test]
async fn a_scaled_out_service_supersedes_the_cached_document_at_a_new_epoch() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, PORTS_SCALE_OUT_SUPERSEDES).await;

    let app_did =
        submit_and_adopt(&mut supervisor_node, "scale-out-inst", inventory_json.clone(), 1).await;

    let outside_caller = Identity::generate().unwrap();
    let outside_caller_did = substrate::derive_did_key(&outside_caller.public_key());
    let grant = resolve_grant(&supervisor_owner, &outside_caller_did, &app_did);
    let fetcher = outside_caller_fetcher(&supervisor_node.registry_url, &outside_caller, grant);
    let app_did_typed = AppDid::new(app_did.clone());
    let service_name = LogicalServiceName::new("backend");

    let first = fetcher.fetch(&app_did_typed, &service_name).await.expect("first fetch failed");
    assert_eq!(first.document.members.len(), 1);
    let first_epoch = first.document.epoch;

    // The caller side: register the first document and resolve through it,
    // the same path a real outside caller uses -- not just compare the raw
    // fetched documents, which would say nothing about `LogicalResolver`'s
    // own cache being superseded.
    let resolver = LogicalResolver::new(Arc::new(StaticInventory::new()));
    let key = register_verified(&resolver, &first, &app_did_typed, None)
        .expect("register_verified failed for the first document");
    assert_eq!(resolver.resolve_all(&key).unwrap().members.len(), 1);

    // Scale out: resubmit at 2 replicas.
    let manifest = service_manifest(2);
    let plan_json = compiled_plan_json(&manifest, "scale-out-inst").await;
    supervisor_node
        .substrate_client
        .request("supervisor", "submit", submission("scale-out-inst", plan_json, inventory_json, 1))
        .await
        .expect("resubmit failed");

    let deadline = Instant::now() + Duration::from_secs(30);
    let second = loop {
        let doc = fetcher.fetch(&app_did_typed, &service_name).await.expect("second fetch failed");
        if doc.document.epoch != first_epoch {
            break doc;
        }
        assert!(Instant::now() < deadline, "the topology epoch never advanced after a scale-out");
        time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(second.document.members.len(), 2);
    assert!(second.document.epoch.0 > first_epoch.0);

    // Re-registering the second document must supersede the first in the
    // caller's own resolver, under the same key, with no further fetch.
    let key2 = register_verified(&resolver, &second, &app_did_typed, None)
        .expect("register_verified failed for the second document");
    assert_eq!(key2, key, "a scale-out must not change which key a caller resolves under");
    assert_eq!(resolver.resolve_all(&key).unwrap().members.len(), 2);

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

/// Reference scenario step 7, matrix row 4's first half: an already-cached
/// document still routes after the supervisor that signed it goes down --
/// the property that proves the supervisor is off the availability path
/// after the first fetch.
#[tokio::test]
async fn a_cached_document_still_routes_after_the_supervisor_is_down() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) = boot_pair(
        &supervisor_owner,
        &managed_owner,
        PORTS_CACHED_DOCUMENT_SURVIVES_SUPERVISOR_DOWN,
    )
    .await;

    let app_did =
        submit_and_adopt(&mut supervisor_node, "cached-survives-inst", inventory_json, 1).await;

    let outside_caller = Identity::generate().unwrap();
    let outside_caller_did = substrate::derive_did_key(&outside_caller.public_key());
    let grant = resolve_grant(&supervisor_owner, &outside_caller_did, &app_did);
    let fetcher = outside_caller_fetcher(&supervisor_node.registry_url, &outside_caller, grant);
    let app_did_typed = AppDid::new(app_did.clone());

    let resolver = LogicalResolver::new(Arc::new(StaticInventory::new()));
    let key = fetch_and_register(
        &fetcher,
        &resolver,
        &app_did_typed,
        &LogicalServiceName::new("backend"),
    )
    .await
    .expect("fetch_and_register failed");

    supervisor_node.teardown().await;

    // No network call reaches the (now-dead) supervisor: this is purely
    // the caller's own in-process cache.
    assert!(resolver.resolve(&key, None).is_ok(), "a cached document must still route");

    managed_node.teardown().await;
}

/// Reference scenario step 7, matrix row 4's **second** half: a caller
/// with no cached document fails cleanly -- not a hang -- when the
/// supervisor it would fetch from is down.
#[tokio::test]
async fn a_caller_with_no_cached_document_fails_cleanly_when_the_supervisor_is_down() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, PORTS_NO_CACHE_FAILS_CLEANLY).await;

    let app_did = submit_and_adopt(&mut supervisor_node, "no-cache-inst", inventory_json, 1).await;

    let outside_caller = Identity::generate().unwrap();
    let outside_caller_did = substrate::derive_did_key(&outside_caller.public_key());
    let grant = resolve_grant(&supervisor_owner, &outside_caller_did, &app_did);
    let fetcher = outside_caller_fetcher(&supervisor_node.registry_url, &outside_caller, grant)
        .with_connect_timeout(Duration::from_secs(5));
    let app_did_typed = AppDid::new(app_did.clone());

    supervisor_node.teardown().await;

    let result = time::timeout(
        Duration::from_secs(20),
        fetcher.fetch(&app_did_typed, &LogicalServiceName::new("backend")),
    )
    .await
    .expect("fetch must fail cleanly, not hang, when the supervisor is down");
    assert!(result.is_err(), "a never-cached caller must not get a partial answer");

    managed_node.teardown().await;
}

/// Reference scenario step 8: a document forged under a different key is
/// rejected.
#[tokio::test]
async fn a_document_forged_under_a_different_key_is_rejected() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, PORTS_FORGED_DOCUMENT_REJECTED).await;

    let app_did =
        submit_and_adopt(&mut supervisor_node, "forged-doc-inst", inventory_json, 1).await;
    let app_did_typed = AppDid::new(app_did.clone());

    let outside_caller = Identity::generate().unwrap();
    let outside_caller_did = substrate::derive_did_key(&outside_caller.public_key());
    let grant = resolve_grant(&supervisor_owner, &outside_caller_did, &app_did);
    let fetcher = outside_caller_fetcher(&supervisor_node.registry_url, &outside_caller, grant);
    let genuine = fetcher
        .fetch(&app_did_typed, &LogicalServiceName::new("backend"))
        .await
        .expect("fetch failed");

    // A forger with no relationship to the app master signs the exact same
    // document content under its own key.
    let forger = Identity::generate().unwrap();
    let forged = genuine.document.clone().sign(&forger).expect("sign failed");
    assert!(
        forged.verify(&app_did_typed).is_err(),
        "a document signed under an unrelated key must not verify against the real app DID"
    );

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}
