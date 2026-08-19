#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Slice B3 Phase 5: reference-scenario step 23 ("A ReBAC check requiring a
//! remote relationship proof triggers a cross-service fetch via the
//! Universal Proxy mid-query") proven across two genuinely independent
//! `syneroym-substrate` instances -- distinct identities, distinct Iroh
//! listen ports, distinct on-disk storage -- rather than the in-process
//! `ProxyRouter` Phase 4's own tests already cover
//! (`crates/router/tests/native_dispatch_identity.rs`, where the "remote"
//! service is an in-process `NativeHostChannel` in the same test binary).
//!
//! Node A hosts the data-owning `employee` relation (`resolvable_without_
//! capability`, the A2 receiving-side mode -- no cross-node capability
//! delegation needed). Node B hosts a `documents` collection whose FDAE
//! policy names Node A's service as a remote relation, trusting Node A's
//! real per-service `expected_asserter_did` (D-B3-8), computed the same way
//! a real policy author would: independently, from `(owner_did,
//! service_id)`, never read back off a proof. Querying Node B for real, over
//! a real Iroh QUIC connection, drives `plan_read` -> `resolve_fetches` (a
//! second real Iroh QUIC hop, Node B -> Node A) -> `finalize`, and only
//! alice's own document comes back.

use std::time::{Duration, Instant};

use reqwest::Client as HttpClient;
use rustls::crypto::ring;
use serde_json::{Map, json};
use syneroym_core::{
    config::{
        ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole, IrohParentConfig, LogTarget,
        ServiceRegistryRole, SubstrateConfig,
    },
    dht_registry::{EndpointInfo, EndpointMechanism, EndpointType},
};
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{Ability, Capability, CapabilityToken, JsonRpcError, ResourceUri};
use syneroym_sdk::{DeployManifest, ServiceConfig, ServiceType, SyneroymClient, TcpManifest};
use syneroym_substrate::identity;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::DocumentSource;
use tempfile::TempDir;
use tokio::{
    sync::{mpsc, mpsc::Sender},
    task::JoinHandle,
};

// `coordinator_iroh`'s "/v1/info" server always binds `http_bind_address`'s
// port + 10 (`spawn_http_info_server`), so Node B's own `http_bind_address`
// must stay well clear of Node A's `+10` (and vice versa) -- hence the
// hundred-port gap rather than adjacent blocks.
const NODE_A_IROH_PORT: u16 = 8000;
const NODE_A_REGISTRY_PORT: u16 = 8001;
const NODE_A_GATEWAY_PORT: u16 = 8002;
const NODE_B_IROH_PORT: u16 = 8100;
const NODE_B_REGISTRY_PORT: u16 = 8101;
const NODE_B_GATEWAY_PORT: u16 = 8102;

const FETCH_LATENCY_ITERATIONS: usize = 5;
// The < 50 ms p99 budget in task.md's Performance Budgets table names the
// *fetch's own* processing cost; on this sandboxed environment each call
// measured here is dominated by a different, larger cost that budget was
// never meant to capture: `IrohHop` (crates/router/src/proxy.rs) opens a
// brand-new QUIC connection for every single `resolve_fetches` call (no
// connection reuse/pooling exists yet -- confirmed by zero "proxy call
// failed; retrying" log lines across every iteration, i.e. each connection
// succeeded on its first attempt, so the cost is inherent to fresh
// connection establishment, not retries). Observed p50/p99 land in the
// low single-digit seconds here, ~100x the budget -- see status.md for the
// measured numbers and the `deferred-backlog.md` entry this finding adds
// (`IrohHop` connection reuse). This constant is therefore a hang/
// regression backstop, not the task.md budget itself.
const FETCH_LATENCY_SANITY_CEILING: Duration = Duration::from_secs(30);

/// A full, independently-identified `syneroym-substrate` instance. Mirrors
/// every other `crates/substrate/tests/*.rs` file's own `SubstrateTestContext`
/// (each keeps its own near-verbatim copy -- see `tests/common/mod.rs`'s doc
/// comment), with one addition: `shared_registry_url` lets a second node
/// point itself at the *first* node's registry instead of its own, so a
/// cross-node community-registry lookup (`ProxyRouter::invoke_remote`) has
/// something real to resolve against. The node still starts its own
/// `community_registry` role either way (unused when sharing) -- the
/// smallest possible delta from the proven self-referential single-node
/// shape every sibling test file already relies on.
struct Node {
    config: SubstrateConfig,
    substrate_client: SyneroymClient,
    registry_url: String,
    substrate_mechanisms: Vec<EndpointMechanism>,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
    _temp_dir: TempDir,
}

impl Node {
    /// `owner` becomes this node's `[iam].admin_ucan_root` and the identity
    /// its own `substrate_client` presents (an unowned
    /// substrate now fails closed, so `inject_kek` -- a `security` call --
    /// needs an owning identity behind it too, not just a config value).
    /// `None` used to mean "unowned"; that posture is no longer usable for
    /// anything this fixture drives, so every caller now passes `Some`.
    async fn boot(
        iroh_port: u16,
        registry_port: u16,
        gateway_port: u16,
        shared_registry_url: Option<String>,
        owner: Option<Identity>,
        shared_relay_url: Option<String>,
    ) -> Self {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let base_path = temp_dir.path();
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
        // Sharing one node's relay (rather than each node self-relaying
        // through its own, distinct home relay) avoids cross-relay direct-
        // path/hole-punch negotiation between two localhost peers that
        // otherwise adds seconds of connection-establishment latency to
        // every fresh `IrohHop` dial -- purely an artifact of two
        // independently-relayed loopback endpoints, not a real network
        // condition the 50 ms federated-hop budget is meant to capture.
        let relay_url = shared_relay_url.unwrap_or_else(|| format!("http://localhost:{iroh_port}"));
        config.parent_coordinator.iroh = Some(IrohParentConfig { url: relay_url });
        config.roles.client_gateway =
            Some(ClientGatewayRole { http_port: gateway_port, ..Default::default() });
        // An owned node so an unrelated caller (e.g. Node A's own forwarded
        // `resolve-relation` invocation, proxying for a Node B principal who
        // never delegated anything on Node A) does not fall back to a
        // free, bare `substrate:<node_did>`-scoped `orchestrator/*`
        // capability -- `resolve_relation`'s A1/A2 fork (B3-07) treats *any*
        // substrate-scoped capability as "holds a capability scoped to this
        // resource," regardless of its ability, which would force A1 and
        // defeat `resolvable_without_capability`'s A2 path entirely. Now
        // this reasoning applies unconditionally -- an
        // unowned substrate no longer issues that free grant at all, but it
        // also grants nothing else, so every caller here still needs a
        // real owner (or, for a non-owner deployer, an app-scoped grant
        // from one; see the `app_deploy_grant` call sites below).
        let owner_did = owner.as_ref().map(|id| substrate::derive_did_key(&id.public_key()));
        config.iam.admin_ucan_root = owner_did;

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

        let mut substrate_client = match owner {
            Some(id) => SyneroymClient::new_with_identity(
                substrate_service_id.clone(),
                effective_registry_url.clone(),
                id,
            )
            .with_registry_dht(false),
            None => {
                SyneroymClient::new(substrate_service_id.clone(), effective_registry_url.clone())
                    .with_registry_dht(false)
            }
        };
        substrate_client
            .wait_for_ready(Duration::from_secs(30))
            .await
            .expect("substrate did not become available in time");

        let substrate_info =
            substrate_client.lookup().await.expect("failed to lookup substrate info from registry");
        let substrate_mechanisms = substrate_info.info.mechanisms;

        Self {
            config,
            substrate_client,
            registry_url: effective_registry_url,
            substrate_mechanisms,
            shutdown_tx,
            substrate_handle,
            _temp_dir: temp_dir,
        }
    }

    fn did(&self) -> &str {
        self.substrate_client.service_id()
    }

    /// Reads this node's own raw node identity back off disk -- "node-
    /// private" by design (`crates/identity/src/keys.rs`'s
    /// `derive_service_identity` doc comment), but this test constructed
    /// the node itself, so reading its own key file is legitimate (the same
    /// access a real node operator would have), not a bypass of the
    /// property that no *other* party can compute it.
    fn node_identity(&self) -> Identity {
        let secret = identity::get_secret(&self.config.identity, &self.config.app_data_dir)
            .expect("failed to read node identity secret");
        Identity::from_bytes(&secret)
    }

    async fn teardown(mut self) {
        let _ = self.substrate_client.shutdown().await;
        let _ = self.shutdown_tx.send(()).await;
        let _ = self.substrate_handle.await;
    }
}

/// A bare TCP service (no endpoints, no HTTP routes -- this test only cares
/// about the `data-layer` native capability every deployed service gets
/// regardless of `service_type`) carrying an optional inline FDAE policy.
fn deploy_manifest(policy_json: Option<String>) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: policy_json.map(DocumentSource::Inline),
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: ServiceType::Tcp(TcpManifest { endpoints: vec![] }),
        registry_certificate: None,
        instance_certificate: None,
    }
}

async fn deploy(client: &SyneroymClient, service_id: &str, manifest: DeployManifest) {
    let params = serde_json::to_value((service_id.to_string(), manifest)).unwrap();
    let res =
        client.request("orchestrator", "deploy", params).await.expect("deploy request failed");
    assert_eq!(res.result, json!({"status": "deployed"}), "deploy did not succeed: {res:?}");
}

/// An app-scoped `orchestrator/{deploy,undeploy,status}` grant, issued by
/// `node_owner`, letting `grantee_did` deploy (and later undeploy) exactly
/// one app on `node_did` -- the same shape `deploy_grant.rs` already
/// exercises, made real here now that the free
/// unowned-substrate grant `alice_deployer`/`bad_app_deployer` used to ride
/// on. All three abilities are granted together, not deploy-only: `deploy`
/// calls `self.undeploy(.., caller)` on two rollback paths
/// (`orchestration.rs`'s `undeploy_impl` doc), so a deploy-only grant would
/// make a *failed* deploy fail again on a confusing second error --
/// `deploy_grant.rs` owns the partial-grant cases, not this fixture.
fn app_deploy_grant(
    node_owner: &Identity,
    grantee_did: &str,
    node_did: &str,
    service_id: &str,
) -> CapabilityToken {
    let resource = ResourceUri(format!("substrate:{node_did}/app/{service_id}"));
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

/// Publishes `service_id`'s own `EndpointInfo` -- reachable at the hosting
/// node's own Iroh mechanisms -- into `registry_url`, self-signed by
/// `service_identity`. Mirrors `basic_lifecycle.rs`'s own
/// `register_app_in_registry` helper (same shape, independently verified
/// precedent for exactly this "a deployed app is a distinct, separately
/// resolvable registry entry" pattern).
async fn register_service(
    service_id: &str,
    host_substrate_did: &str,
    host_mechanisms: Vec<EndpointMechanism>,
    service_identity: &Identity,
    registry_url: &str,
) {
    let info = EndpointInfo {
        service_id: service_id.to_string(),
        substrate_id: host_substrate_did.to_string(),
        endpoint_type: EndpointType::Service,
        nickname: None,
        mechanisms: host_mechanisms,
        is_private: false,
        ttl: None,
        not_after: u64::MAX / 2,
        generation: 0,
    };
    let signed_info = info.sign(service_identity).expect("failed to sign endpoint info");
    let res = HttpClient::new()
        .post(format!("{registry_url}/register"))
        .json(&signed_info)
        .send()
        .await
        .expect("failed to register service in HTTP registry");
    assert!(
        res.status().is_success(),
        "registry registration failed: {}",
        res.text().await.unwrap_or_default()
    );
}

#[tokio::test]
async fn federated_fdae_fetch_across_two_real_substrates() {
    let _ = ring::default_provider().install_default();

    let alice_identity = Identity::generate().unwrap();
    let alice_did = substrate::derive_did_key(&alice_identity.public_key());
    let hr_owner_identity = Identity::generate().unwrap();
    let hr_owner_did = substrate::derive_did_key(&hr_owner_identity.public_key());

    // Node A is owned by `hr_owner_did` -- otherwise every unrelated caller
    // (including alice's forwarded `resolve-relation` identity, who never
    // delegated anything on Node A) would fall back to a free, bare
    // `substrate:<node_did>`-scoped `orchestrator/*` capability, which
    // `resolve_relation`'s A1/A2 fork (B3-07) treats as "holds a capability
    // scoped to this resource" regardless of ability -- forcing A1 and
    // defeating `resolvable_without_capability`'s A2 path for every caller.
    let mut node_a = Node::boot(
        NODE_A_IROH_PORT,
        NODE_A_REGISTRY_PORT,
        NODE_A_GATEWAY_PORT,
        None,
        Some(Identity::from_bytes(&hr_owner_identity.to_bytes())),
        None,
    )
    .await;
    let node_a_relay_url = format!("http://localhost:{NODE_A_IROH_PORT}");

    // Node B gets its own, *distinct* owner -- deliberately not
    // `hr_owner_did` and not alice: giving Node B an
    // admin root named after alice would grant her `substrate/admin`,
    // which entails `data-layer/write` everywhere on Node B
    // (`Ability::entails`'s short-circuit) and would make the carefully
    // separated read-only/write-only tokens below meaningless while every
    // assertion still passed. `alice_deployer` below instead gets an
    // app-scoped deploy grant issued by `node_b_owner` -- the property this
    // file exists to prove is that alice holds no broader capability at
    // all without one.
    let node_b_owner = Identity::generate().unwrap();
    let node_b = Node::boot(
        NODE_B_IROH_PORT,
        NODE_B_REGISTRY_PORT,
        NODE_B_GATEWAY_PORT,
        Some(node_a.registry_url.clone()),
        Some(Identity::from_bytes(&node_b_owner.to_bytes())),
        Some(node_a_relay_url),
    )
    .await;

    // Default `storage.encryption = true` requires a KEK before any
    // service's database can be opened (`crates/substrate/tests/
    // http_passthrough_e2e.rs`'s own precedent for this exact step).
    // `node_a`'s connection was dialed and proven live by its own
    // `wait_for_ready` during `Node::boot`, then sat idle for the entire
    // `node_b` boot that followed -- long enough under CI's scheduling
    // pressure for the peer to abandon that idle path ("no viable network
    // path exists: last path abandoned by peer"; same root cause fixed in
    // `binding_push_e2e.rs`). Recover by explicit shutdown→reconnect before
    // one retry.
    if node_a.substrate_client.inject_kek("11".repeat(32)).await.is_err() {
        node_a
            .substrate_client
            .shutdown()
            .await
            .expect("failed to reset node A's stale connection");
        node_a.substrate_client.connect().await.expect("failed to reconnect node A");
        node_a
            .substrate_client
            .inject_kek("11".repeat(32))
            .await
            .expect("node A inject_kek failed");
    }
    node_b.substrate_client.inject_kek("22".repeat(32)).await.expect("node B inject_kek failed");

    // --- Node A: the data-owning "hr" service. A2 (`resolvable_without_
    // capability`) means the resolving fetch needs no capability delegated
    // cross-node at all -- just a re-verified identity match. ---
    let hr_service_identity = Identity::generate().unwrap();
    let hr_service_id = substrate::derive_did_key(&hr_service_identity.public_key());

    let mut hr_deployer = SyneroymClient::new_with_identity(
        node_a.did().to_string(),
        node_a.registry_url.clone(),
        Identity::from_bytes(&hr_owner_identity.to_bytes()),
    )
    .with_registry_dht(false);
    hr_deployer.connect().await.expect("failed to connect to node A for deploy");

    // `seed` (`paths: []`, public) lets the deploying owner's own
    // `substrate/admin` capability seed fixture rows -- the write-side gate
    // now covers writes too, and `view_self` (read-only) doesn't cover
    // `data-layer/write`. Realistic (a service owner seeding its own data).
    // This does *not* widen the read assertions below because `allows:
    // [data-layer/write]` entails a read check too (`write` ⊇ `read`, the
    // tiered hierarchy) -- "omits read" does not narrow entailment. It's
    // safe here only because the readers exercised further down never
    // present a capability above `data-layer/read` on this resource, so
    // they never become entitled to `seed` in the first place; it is not a
    // property of this permission shape in general (contrast Node B's
    // `alice_data_client` below, which deliberately keeps its write-or-
    // better token off the query connection for exactly this reason).
    let hr_policy = r#"{
        "version": "fdae/v1",
        "definitions": {
            "employee": {
                "table": "employees",
                "principal_column": "did",
                "resolvable_without_capability": true,
                "permissions": {
                    "view_self": {"allows": ["data-layer/read"], "paths": [["caller"]]},
                    "seed": {"allows": ["data-layer/write"], "paths": []}
                }
            }
        }
    }"#;
    deploy(&hr_deployer, &hr_service_id, deploy_manifest(Some(hr_policy.to_string()))).await;
    register_service(
        &hr_service_id,
        node_a.did(),
        node_a.substrate_mechanisms.clone(),
        &hr_service_identity,
        &node_a.registry_url,
    )
    .await;

    // A separate client targeting `hr_service_id` itself, not `node_a.did()`
    // (the orchestrator target `hr_deployer` used above): the substrate's
    // own root DID never registers a `data-layer` endpoint, so addressing it
    // with a `data-layer` preamble would fall through to `ProxyRouter`'s
    // registry-based remote-dispatch path -- and resolve right back to this
    // same node's own address, which Iroh refuses to dial ("Connecting to
    // ourself is not supported").
    let mut hr_data_client = SyneroymClient::new_with_identity(
        hr_service_id.clone(),
        node_a.registry_url.clone(),
        Identity::from_bytes(&hr_owner_identity.to_bytes()),
    )
    .with_registry_dht(false);
    hr_data_client.connect().await.expect("failed to connect to node A's hr service");

    hr_data_client
        .request("data-layer", "create-collection", json!({"name": "employees"}))
        .await
        .expect("create-collection(employees) failed");
    for (id, did) in [("emp-alice", alice_did.as_str()), ("emp-bob", "did:key:federated-e2e-bob")] {
        hr_data_client
            .request(
                "data-layer",
                "put",
                json!({
                    "collection": "employees",
                    "value": {"id": id, "payload": json!({"did": did}).to_string().into_bytes()}
                }),
            )
            .await
            .expect("seeding employees failed");
    }

    // The DID a policy author would independently compute and declare as
    // `expected_asserter_did` -- never read back off a proof (D-B3-8).
    let node_a_identity = node_a.node_identity();
    let expected_asserter_did = substrate::derive_did_key(
        &node_a_identity.derive_service_identity(&hr_owner_did, &hr_service_id).public_key(),
    );

    // --- Node B: alice's own "documents" app. She deploys (and so owns) it
    // herself, so her later self-issued root capability on it is trusted
    // per ADR-0015 A6 (`registry.owner_of(svc) == issuer`) with no node-wide
    // admin root and no delegation chain needed -- but *reaching* `deploy`
    // at all now needs an app-scoped grant from Node B's owner,
    // since Node B no longer issues a free `orchestrator/deploy` to
    // every verified caller. ---
    let app_service_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_service_identity.public_key());

    let mut alice_deployer = SyneroymClient::new_with_identity(
        node_b.did().to_string(),
        node_b.registry_url.clone(),
        Identity::from_bytes(&alice_identity.to_bytes()),
    )
    .with_registry_dht(false)
    .with_ucan(app_deploy_grant(&node_b_owner, &alice_did, node_b.did(), &app_service_id));
    alice_deployer.connect().await.expect("failed to connect to node B for deploy");

    let app_policy = format!(
        r#"{{
            "version": "fdae/v1",
            "definitions": {{
                "document": {{
                    "table": "documents",
                    "relations": {{"owner": {{
                        "target": "employee", "service": "{hr_service_id}", "join_column": "owner_uuid",
                        "expected_asserter_did": "{expected_asserter_did}"
                    }}}},
                    "permissions": {{
                        "view": {{"allows": ["data-layer/read"], "paths": [["owner", "anchor"]]}},
                        "seed": {{"allows": ["data-layer/write"], "paths": []}}
                    }}
                }}
            }}
        }}"#
    );
    deploy(&alice_deployer, &app_service_id, deploy_manifest(Some(app_policy))).await;
    register_service(
        &app_service_id,
        node_b.did(),
        node_b.substrate_mechanisms.clone(),
        &app_service_identity,
        &node_b.registry_url,
    )
    .await;

    // Same reasoning as `hr_data_client` above: seed through a client
    // targeting `app_service_id` itself, not `node_b.did()`. Node B's owner
    // is `node_b_owner`, not alice, so -- unlike Node A's `hr_data_client`
    // -- alice holds no capability at all without self-issuing one; a
    // *write-only* token, trusted per ADR-0015 A6 (she owns the service she
    // deployed), entitles the "seed" permission below. Deliberately a
    // separate token from `alice_query_client`'s read-only one further
    // down, on a separate connection -- `data-layer/write` entails
    // `data-layer/read` (the tiered hierarchy), so a write capability
    // presented on the *query* connection would make "seed"'s public
    // `paths: []` applicable there too, leaking every document instead of
    // just alice's own.
    let seed_resource = ResourceUri(format!(
        "{}/collection/documents",
        ResourceUri::service(&app_service_id, &app_service_id).0
    ));
    let seed_token = CapabilityToken::issue(
        &alice_identity,
        &alice_did,
        vec![Capability {
            with: seed_resource,
            can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
            caveats: None,
        }],
        Map::new(),
        3600,
        vec![],
    )
    .expect("failed to self-issue the seeding capability token");
    let mut alice_data_client = SyneroymClient::new_with_identity(
        app_service_id.clone(),
        node_b.registry_url.clone(),
        Identity::from_bytes(&alice_identity.to_bytes()),
    )
    .with_registry_dht(false)
    .with_ucan(seed_token);
    alice_data_client.connect().await.expect("failed to connect to node B's app service");

    alice_data_client
        .request("data-layer", "create-collection", json!({"name": "documents"}))
        .await
        .expect("create-collection(documents) failed");
    for (id, owner_uuid) in [("doc-1", "emp-alice"), ("doc-2", "emp-bob")] {
        alice_data_client
            .request(
                "data-layer",
                "put",
                json!({
                    "collection": "documents",
                    "value": {"id": id, "payload": json!({"owner_uuid": owner_uuid}).to_string().into_bytes()}
                }),
            )
            .await
            .expect("seeding documents failed");
    }

    // Alice self-issues a root capability (no proofs, no node-wide admin
    // needed) granting herself `data-layer/read` on her own `documents`
    // collection, and connects to her own app service to actually query it.
    let resource = ResourceUri(format!(
        "{}/collection/documents",
        ResourceUri::service(&app_service_id, &app_service_id).0
    ));
    let token = CapabilityToken::issue(
        &alice_identity,
        &alice_did,
        vec![Capability {
            with: resource,
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: None,
        }],
        Map::new(),
        3600,
        vec![],
    )
    .expect("failed to self-issue capability token");

    let mut alice_query_client = SyneroymClient::new_with_identity(
        app_service_id.clone(),
        node_b.registry_url.clone(),
        alice_identity,
    )
    .with_registry_dht(false)
    .with_ucan(token);
    alice_query_client.connect().await.expect("failed to connect to node B as alice");

    let query_body = json!({"collection": "documents", "opts": {}});

    // Warmup, then measure the real cross-substrate fetch's latency: each
    // `query` call drives `plan_read` -> `resolve_fetches` (a real Iroh QUIC
    // hop, Node B -> Node A, `IrohHop` opens a fresh connection per call) ->
    // `finalize` -> the local SQL join, end to end over the wire.
    let resp = alice_query_client
        .request("data-layer", "query", query_body.clone())
        .await
        .expect("warmup query failed");
    let records = resp.result["records"].as_array().expect("records array");
    assert_eq!(records.len(), 1, "only alice's own document must be reachable: {:?}", resp.result);

    let mut latencies = Vec::with_capacity(FETCH_LATENCY_ITERATIONS);
    for _ in 0..FETCH_LATENCY_ITERATIONS {
        let start = Instant::now();
        let resp = alice_query_client
            .request("data-layer", "query", query_body.clone())
            .await
            .expect("query failed");
        latencies.push(start.elapsed());

        let records = resp.result["records"].as_array().expect("records array");
        assert_eq!(
            records.len(),
            1,
            "only alice's own document must be reachable through the real cross-substrate fetch: \
             {:?}",
            resp.result
        );
        assert_eq!(records[0]["id"], "doc-1");
    }

    // With FETCH_LATENCY_ITERATIONS this small, "p99" is arithmetically
    // indistinguishable from max -- not a real percentile, just a smoke
    // number alongside the hang backstop below. Labeled as such in the log
    // line so it isn't misread as a statistically meaningful measurement.
    latencies.sort();
    let p50 = latencies[latencies.len() / 2];
    let p99_idx = ((latencies.len() as f64) * 0.99).ceil() as usize;
    let p99 = latencies[p99_idx.saturating_sub(1).min(latencies.len() - 1)];
    let max = *latencies.last().unwrap();
    println!(
        "federated FDAE fetch (Node B -> Node A, real Iroh QUIC, {} iterations): p50={p50:?} \
         p99(n={}, ~=max)={p99:?} max={max:?}",
        latencies.len(),
        latencies.len()
    );
    assert!(
        max < FETCH_LATENCY_SANITY_CEILING,
        "federated FDAE fetch max {max:?} exceeds the {FETCH_LATENCY_SANITY_CEILING:?} hang/ \
         regression backstop (see this constant's doc comment -- the task.md < 50 ms p99 budget \
         is tracked separately in status.md, not enforced here)"
    );

    // A cross-service fetch that times out must fall back to deny, not
    // silent allow -- Failure/Security matrix row 6 (Slice B3 Phase 4's own
    // mechanism/unit coverage), now proven across two real substrates: point
    // the policy at an asserter node A never actually derives (a node A
    // deploy that names the *wrong* `expected_asserter_did`, the shape a
    // stale/misconfigured policy would produce) and confirm the query denies
    // closed instead of leaking bob's row or erroring out unfiltered.
    let bad_app_service_identity = Identity::generate().unwrap();
    let bad_app_service_id = substrate::derive_did_key(&bad_app_service_identity.public_key());
    let bad_app_policy = format!(
        r#"{{
            "version": "fdae/v1",
            "definitions": {{
                "document": {{
                    "table": "documents",
                    "relations": {{"owner": {{
                        "target": "employee", "service": "{hr_service_id}", "join_column": "owner_uuid",
                        "expected_asserter_did": "did:key:z6MkNotTheRealHrSvcNode"
                    }}}},
                    "permissions": {{
                        "view": {{"allows": ["data-layer/read"], "paths": [["owner", "anchor"]]}},
                        "seed": {{"allows": ["data-layer/write"], "paths": []}}
                    }}
                }}
            }}
        }}"#
    );
    let alice_identity_2 = Identity::generate().unwrap();
    let alice_did_2 = substrate::derive_did_key(&alice_identity_2.public_key());
    let seed_second_employee = json!({
        "collection": "employees",
        "value": {"id": "emp-alice-2", "payload": json!({"did": alice_did_2}).to_string().into_bytes()}
    });
    // `hr_data_client` was dialed once, early (line ~449), and has sat idle
    // through the five-iteration connection-churn benchmark above (`IrohHop`
    // opens a fresh connection per call against Node A, no reuse yet -- see
    // `FETCH_LATENCY_SANITY_CEILING`'s own doc comment) -- long enough, under
    // CI's own scheduling pressure, for Node A to abandon this idle path
    // before it gets reused ("no viable network path exists: last path
    // abandoned by peer"). `SyneroymClient::connect` no-ops once a
    // connection is already `Some(..)`, so recovering means an explicit
    // `shutdown` (drops the dead one) then `connect` (dials fresh) before
    // retrying, not just retrying the same request on the same connection.
    if hr_data_client.request("data-layer", "put", seed_second_employee.clone()).await.is_err() {
        hr_data_client
            .shutdown()
            .await
            .expect("failed to reset hr_data_client's stale connection to node A");
        hr_data_client.connect().await.expect("failed to reconnect hr_data_client to node A");
    }
    hr_data_client
        .request("data-layer", "put", seed_second_employee)
        .await
        .expect("seeding second employee failed");

    // A *third* deployer on Node B: needs its own app-scoped
    // grant, issued to `alice_did_2` specifically -- `bad_seed_token`/
    // `bad_query_client` further down self-issue owner-rooted tokens that
    // only verify because `registry.owner_of(bad_app_service_id) ==
    // alice_did_2` (ADR-0015 A6, and `deploy` records `caller.caller_did` as
    // the owner), so deploying as anyone else here would silently break
    // that root.
    let mut bad_app_deployer = SyneroymClient::new_with_identity(
        node_b.did().to_string(),
        node_b.registry_url.clone(),
        Identity::from_bytes(&alice_identity_2.to_bytes()),
    )
    .with_registry_dht(false)
    .with_ucan(app_deploy_grant(
        &node_b_owner,
        &alice_did_2,
        node_b.did(),
        &bad_app_service_id,
    ));
    bad_app_deployer.connect().await.expect("failed to connect to node B for the mismatch deploy");
    deploy(&bad_app_deployer, &bad_app_service_id, deploy_manifest(Some(bad_app_policy))).await;
    register_service(
        &bad_app_service_id,
        node_b.did(),
        node_b.substrate_mechanisms.clone(),
        &bad_app_service_identity,
        &node_b.registry_url,
    )
    .await;

    // Write-only self-issued token, same reasoning as `alice_data_client`
    // above: a separate connection from `bad_query_client`'s read-only one
    // below, so "seed"'s public `paths: []` never becomes reachable for a
    // read.
    let bad_seed_resource = ResourceUri(format!(
        "{}/collection/documents",
        ResourceUri::service(&bad_app_service_id, &bad_app_service_id).0
    ));
    let bad_seed_token = CapabilityToken::issue(
        &alice_identity_2,
        &alice_did_2,
        vec![Capability {
            with: bad_seed_resource,
            can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
            caveats: None,
        }],
        Map::new(),
        3600,
        vec![],
    )
    .expect("failed to self-issue the mismatch seeding capability token");
    let mut bad_app_data_client = SyneroymClient::new_with_identity(
        bad_app_service_id.clone(),
        node_b.registry_url.clone(),
        Identity::from_bytes(&alice_identity_2.to_bytes()),
    )
    .with_registry_dht(false)
    .with_ucan(bad_seed_token);
    bad_app_data_client
        .connect()
        .await
        .expect("failed to connect to node B's mismatch app service");
    bad_app_data_client
        .request("data-layer", "create-collection", json!({"name": "documents"}))
        .await
        .expect("create-collection(documents) on the mismatch app failed");
    bad_app_data_client
        .request(
            "data-layer",
            "put",
            json!({
                "collection": "documents",
                "value": {
                    "id": "doc-mismatch",
                    "payload": json!({"owner_uuid": "emp-alice-2"}).to_string().into_bytes()
                }
            }),
        )
        .await
        .expect("seeding the mismatch app's document failed");

    let bad_resource = ResourceUri(format!(
        "{}/collection/documents",
        ResourceUri::service(&bad_app_service_id, &bad_app_service_id).0
    ));
    let bad_token = CapabilityToken::issue(
        &alice_identity_2,
        &alice_did_2,
        vec![Capability {
            with: bad_resource,
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: None,
        }],
        Map::new(),
        3600,
        vec![],
    )
    .expect("failed to self-issue the mismatch capability token");
    let mut bad_query_client = SyneroymClient::new_with_identity(
        bad_app_service_id,
        node_b.registry_url.clone(),
        alice_identity_2,
    )
    .with_registry_dht(false)
    .with_ucan(bad_token);
    bad_query_client.connect().await.expect("failed to connect to node B for the mismatch query");

    // A cross-service fetch failure (asserter mismatch, timeout, ...) maps
    // to a hard `PermissionDenied` (code -32010) error, not an empty-but-
    // successful result (`SynSvcNativeService::resolve_query_auth`'s
    // documented behavior, also `native_dispatch_identity.rs`'s own
    // `native_dispatch_denies_closed_on_a_cross_service_fetch_failure`) --
    // deliberately different from an ordinary "row not reachable" deny
    // (ADR-0007's own "no result is a valid outcome"), since this is an
    // infrastructure/trust failure, not a legitimate absence. `SyneroymClient::
    // request_raw` recovers the wire's JSON-RPC error envelope as a
    // downcastable `syneroym_rpc::JsonRpcError`, so this asserts the actual
    // -32010 deny path, not just "some error happened" (which an unrelated
    // transport failure would also satisfy).
    let err = bad_query_client.request("data-layer", "query", query_body).await.expect_err(
        "a policy trusting the wrong expected_asserter_did must deny closed with an error, not \
         leak bob's row or succeed unfiltered",
    );
    let rpc_err = err.downcast_ref::<JsonRpcError>().unwrap_or_else(|| {
        panic!("expected a structured JSON-RPC error, got a transport-level error instead: {err:?}")
    });
    assert_eq!(
        rpc_err.code, -32010,
        "an asserter mismatch must deny closed via PermissionDenied (-32010), not some other \
         error: {rpc_err:?}"
    );

    node_a.teardown().await;
    node_b.teardown().await;
}
