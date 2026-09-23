#![allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
//! Tier 2 of the logical discovery overlay (ADR-0022 §3), proven across two
//! genuinely independent `syneroym-substrate` instances -- the reference
//! scenario, steps 3 through 8: a caller outside the app instance
//! fetches the signed topology document, verifies it, routes from it, and
//! keeps routing after the supervisor that signed it goes away.
//!
//! Both nodes come from `common::SubstrateNode`: a supervisor node hosting
//! the registry and a managed node publishing into it through a shared relay.
//! `common::serial_guard` keeps this binary's tests from running substrate
//! stacks at once. `boot_pair` delegates to
//! `common::supervisor_and_managed`; the manifest helpers are local.

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use common::{
    MANAGED_ALIAS, SubstrateNode, compiled_plan_json, submission, supervisor_and_managed,
};
use semver::Version;
use serde_json::{Map, json};
use syneroym_app_orchestration::{
    AppDid, LogicalResolver, LogicalServiceName, SignedTopologyDocument, StaticInventory,
    TopologyFetcher, TopologyVisibility, Visibility,
    models::{
        AppBlueprintId, PlacementSelector, ServiceConfig, ServiceSpec, ServiceType, SubstrateAlias,
        SynAppManifest,
    },
    register_verified,
};
use syneroym_core::dht_registry::RegistryClient;
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};
use syneroym_sdk::{RegistryTopologyFetcher, fetch_and_register};
use tokio::time;

mod common;

#[path = "common/retry.rs"]
mod retry;

/// `supervisor/resolve` on `synapp:<app_did>` (ADR-0022 §5), issued from
/// the supervisor node's own owner -- which the node installs as
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

fn service_manifest_with_vis(
    replicas: u32,
    visibility: Visibility,
    topology_visibility: TopologyVisibility,
) -> SynAppManifest {
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
                assets: None,
                visibility,
            },
            depends_on: vec![],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_ALIAS))),
            replicas,
            sharding_strategy: None,
            schedule: None,
            topology_visibility,
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

/// `replicas`-member manifest, `backend` placed on `MANAGED_ALIAS`.
fn service_manifest(replicas: u32) -> SynAppManifest {
    service_manifest_with_vis(replicas, Visibility::Internal, TopologyVisibility::Restricted)
}

/// Two logical services in **one** app instance, differing only in
/// `topology_visibility` -- the fixture for proving the declaration is per
/// logical service, not per app. `topology_visibility`
/// lives on `ServiceSpec` (per service) rather than the manifest root
/// exactly to make this possible.
fn two_service_manifest_with_topology_vis(
    open_vis: TopologyVisibility,
    restricted_vis: TopologyVisibility,
) -> SynAppManifest {
    let mut services = BTreeMap::new();
    for (name, port, vis) in
        [("open-svc", 41902, open_vis), ("restricted-svc", 41903, restricted_vis)]
    {
        services.insert(
            LogicalServiceName::new(name),
            ServiceSpec {
                config: ServiceConfig {
                    service_type: ServiceType::Tcp,
                    source: format!("127.0.0.1:{port}"),
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
                placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_ALIAS))),
                replicas: 1,
                sharding_strategy: None,
                schedule: None,
                topology_visibility: vis,
            },
        );
    }
    SynAppManifest {
        id: AppBlueprintId::new("syneroym:topology-document-two-service-test-app"),
        version: Version::new(0, 1, 0),
        description: None,
        placement: None,
        services,
        dependencies: BTreeMap::new(),
    }
}

/// Boots a supervisor node and a managed node (the managed one sharing the
/// supervisor's registry and relay), grants the supervisor's own node-wide
/// `orchestrator/deploy` on the managed node, and returns everything a test
/// needs to call `submit`.
async fn boot_pair(
    supervisor_owner: &Identity,
    managed_owner: &Identity,
) -> (SubstrateNode, SubstrateNode, String) {
    supervisor_and_managed(supervisor_owner, managed_owner, 2, MANAGED_ALIAS).await
}

async fn submit_and_adopt_with_manifest(
    supervisor_node: &mut SubstrateNode,
    instance_id: &str,
    inventory_json: String,
    manifest: &SynAppManifest,
) -> String {
    let plan_json = compiled_plan_json(manifest, instance_id).await;
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

    let registry_client =
        RegistryClient::new(false, Some(supervisor_node.registry_url().to_string()));
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

/// Submits and adopts a `replicas`-member instance, returning the app
/// master DID -- the reference scenario's step 1.
async fn submit_and_adopt(
    supervisor_node: &mut SubstrateNode,
    instance_id: &str,
    inventory_json: String,
    replicas: u32,
) -> String {
    let manifest = service_manifest(replicas);
    submit_and_adopt_with_manifest(supervisor_node, instance_id, inventory_json, &manifest).await
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

fn anonymous_outside_caller_fetcher(
    supervisor_registry_url: &str,
    caller: &Identity,
) -> RegistryTopologyFetcher {
    RegistryTopologyFetcher::new(supervisor_registry_url.to_string())
        .with_identity(caller)
        .with_connect_timeout(Duration::from_secs(10))
}

/// Reference scenario steps 3 and 5: an outside caller -- not part of the
/// app instance, holding only `supervisor/resolve` -- fetches the Tier-2
/// document and routes to one of its members.
#[tokio::test]
async fn an_outside_caller_resolves_an_apps_members_and_calls_one() {
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner).await;

    let app_did =
        submit_and_adopt(&mut supervisor_node, "resolve-outside-inst", inventory_json, 2).await;

    let outside_caller = Identity::generate().unwrap();
    let outside_caller_did = substrate::derive_did_key(&outside_caller.public_key());
    let grant = resolve_grant(&supervisor_owner, &outside_caller_did, &app_did);
    let fetcher = outside_caller_fetcher(supervisor_node.registry_url(), &outside_caller, grant);

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
    // is an ordinary registry lookup, unchanged by the topology overlay and
    // already proven at unit scale elsewhere -- this asserts the member
    // this overlay resolved to is a real one, not that Tier 3 itself works.
    let member = resolver.resolve(&key, Some(b"routing-key")).expect("resolve failed");
    let all = resolver.resolve_all(&key).unwrap();
    assert_eq!(all.members.len(), 2, "the two-replica backend must resolve both members");
    assert!(all.members.contains(&member));

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

/// Reference scenario step 4: a document relayed by a party that never
/// contacted the supervisor must verify identically from bytes alone.
#[tokio::test]
async fn a_relayed_document_verifies_for_a_party_that_never_contacted_the_supervisor() {
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner).await;

    let app_did =
        submit_and_adopt(&mut supervisor_node, "relayed-doc-inst", inventory_json, 1).await;

    let outside_caller = Identity::generate().unwrap();
    let outside_caller_did = substrate::derive_did_key(&outside_caller.public_key());
    let grant = resolve_grant(&supervisor_owner, &outside_caller_did, &app_did);
    let fetcher = outside_caller_fetcher(supervisor_node.registry_url(), &outside_caller, grant);

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
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner).await;

    let app_did =
        submit_and_adopt(&mut supervisor_node, "scale-out-inst", inventory_json.clone(), 1).await;

    let outside_caller = Identity::generate().unwrap();
    let outside_caller_did = substrate::derive_did_key(&outside_caller.public_key());
    let grant = resolve_grant(&supervisor_owner, &outside_caller_did, &app_did);
    let fetcher = outside_caller_fetcher(supervisor_node.registry_url(), &outside_caller, grant);
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

/// Reference scenario step 7: an already-cached document still routes after
/// the supervisor that signed it goes down -- the property that proves the
/// supervisor is off the availability path after the first fetch.
#[tokio::test]
async fn a_cached_document_still_routes_after_the_supervisor_is_down() {
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner).await;

    let app_did =
        submit_and_adopt(&mut supervisor_node, "cached-survives-inst", inventory_json, 1).await;

    let outside_caller = Identity::generate().unwrap();
    let outside_caller_did = substrate::derive_did_key(&outside_caller.public_key());
    let grant = resolve_grant(&supervisor_owner, &outside_caller_did, &app_did);
    let fetcher = outside_caller_fetcher(supervisor_node.registry_url(), &outside_caller, grant);
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

/// Reference scenario step 7, the other half: a caller with no cached
/// document fails cleanly -- not a hang -- when the supervisor it would
/// fetch from is down.
#[tokio::test]
async fn a_caller_with_no_cached_document_fails_cleanly_when_the_supervisor_is_down() {
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner).await;

    let app_did = submit_and_adopt(&mut supervisor_node, "no-cache-inst", inventory_json, 1).await;

    let outside_caller = Identity::generate().unwrap();
    let outside_caller_did = substrate::derive_did_key(&outside_caller.public_key());
    let grant = resolve_grant(&supervisor_owner, &outside_caller_did, &app_did);
    let fetcher = outside_caller_fetcher(supervisor_node.registry_url(), &outside_caller, grant)
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
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner).await;

    let app_did =
        submit_and_adopt(&mut supervisor_node, "forged-doc-inst", inventory_json, 1).await;
    let app_did_typed = AppDid::new(app_did.clone());

    let outside_caller = Identity::generate().unwrap();
    let outside_caller_did = substrate::derive_did_key(&outside_caller.public_key());
    let grant = resolve_grant(&supervisor_owner, &outside_caller_did, &app_did);
    let fetcher = outside_caller_fetcher(supervisor_node.registry_url(), &outside_caller, grant);
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

/// An outside caller with no UCAN grant fetches the Tier-2 topology
/// document of an app whose service is declared `open`, and successfully
/// resolves its members.
#[tokio::test]
async fn an_outside_caller_resolves_an_open_apps_members_with_no_ucan_grant() {
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner).await;

    let manifest = service_manifest_with_vis(2, Visibility::Internal, TopologyVisibility::Open);
    let app_did = submit_and_adopt_with_manifest(
        &mut supervisor_node,
        "resolve-open-inst",
        inventory_json,
        &manifest,
    )
    .await;

    let outside_caller = Identity::generate().unwrap();
    let fetcher = anonymous_outside_caller_fetcher(supervisor_node.registry_url(), &outside_caller);

    let app_did_typed = AppDid::new(app_did.clone());
    let resolver = LogicalResolver::new(Arc::new(StaticInventory::new()));
    let key = fetch_and_register(
        &fetcher,
        &resolver,
        &app_did_typed,
        &LogicalServiceName::new("backend"),
    )
    .await
    .expect("fetch_and_register failed for open service");

    let member = resolver.resolve(&key, Some(b"routing-key")).expect("resolve failed");
    let all = resolver.resolve_all(&key).unwrap();
    assert_eq!(all.members.len(), 2, "the two-replica backend must resolve both members");
    assert!(all.members.contains(&member));

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

/// One app instance, two logical services -- `open-svc` declares
/// `topology_visibility = open`, `restricted-svc` declares the default
/// `restricted`. The same ungranted caller gets a different answer for
/// each, proving the declaration is per logical service and not per app.
/// (Previously this test booted a *second, separate* app instance whose
/// single service was `restricted`, which only re-proved a `restricted`
/// app refuses -- long-standing behaviour that is already pinned by
/// `an_outside_caller_resolves_an_open_apps_members_with_no_ucan_grant`'s
/// negative case one test up.)
#[tokio::test]
async fn an_outside_caller_gets_a_different_answer_per_logical_service_in_one_instance() {
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner).await;

    let manifest = two_service_manifest_with_topology_vis(
        TopologyVisibility::Open,
        TopologyVisibility::Restricted,
    );
    let app_did = submit_and_adopt_with_manifest(
        &mut supervisor_node,
        "resolve-mixed-inst",
        inventory_json,
        &manifest,
    )
    .await;

    let outside_caller = Identity::generate().unwrap();
    let fetcher = anonymous_outside_caller_fetcher(supervisor_node.registry_url(), &outside_caller);
    let app_did_typed = AppDid::new(app_did.clone());

    fetcher
        .fetch(&app_did_typed, &LogicalServiceName::new("open-svc"))
        .await
        .expect("the open logical service must resolve for an ungranted caller");

    let err = fetcher
        .fetch(&app_did_typed, &LogicalServiceName::new("restricted-svc"))
        .await
        .unwrap_err();
    // `fetch`'s own `.context("supervisor resolve call failed")` means the
    // JSON-RPC error itself (naming the denial) is further down the chain,
    // not in the top-level `Display` -- `{err:#}` walks the whole chain.
    // `PERMISSION_DENIED_CODE` is `-32010`
    // (`syneroym_rpc::PERMISSION_DENIED_CODE`).
    let full = format!("{err:#}");
    assert!(
        full.contains("-32010") || full.contains("is resolvable by caller"),
        "expected permission denied: {full}"
    );

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}
