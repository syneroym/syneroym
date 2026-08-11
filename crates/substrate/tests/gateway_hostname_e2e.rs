#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Slice S3 of the logical discovery overlay (ADR-0022 §7): the gateway
//! hostname scheme and the routing-key header, proven end to end against
//! two real substrates and a real registry -- the milestone's reference
//! scenario, extended past the SDK/RPC layer to an ordinary HTTP client
//! addressing an app purely by hostname.
//!
//! `Node`, `boot_pair`, `supervisor_role`, `compiled_plan_json`,
//! `node_wide_supervisor_grant`, and `submission` are copied from
//! `topology_document_e2e.rs` (itself copied from
//! `tier1_endpoint_record_e2e.rs`), extended with the two levers this
//! slice's own credential story needs: `grant_resolve_to_node_did` and a
//! `resolve_ucan` token file. The resolve token is minted as a bare
//! `substrate:<supervisor-node-did>` capability (D-S2-7's shape, the same
//! one the same-node gate uses) rather than scoped to one `synapp:<app-did>`
//! -- it can then be minted right after the supervisor node boots, before
//! any app exists, instead of needing a second boot pass once `adopt`
//! reveals the app DID.

use std::{
    collections::BTreeMap,
    io::Write as _,
    net::SocketAddr,
    path::PathBuf,
    time::{Duration, Instant},
};

use reqwest::Client;
use rustls::crypto::ring;
use semver::Version;
use serde_json::{Map, json};
use syneroym_app_orchestration::{
    LocalFilesystemCatalog, LogicalServiceName, compile,
    models::{
        AppBlueprintId, AppInstanceId, PlacementSelector, ServiceConfig, ServiceSpec, ServiceType,
        SubstrateAlias, SynAppManifest,
    },
};
use syneroym_app_supervisor::inventory::SupervisorInventoryEntry;
use syneroym_core::{
    config::{
        ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole, IrohParentConfig, LogTarget,
        ServiceRegistryRole, SubstrateConfig, SupervisorRole,
    },
    dht_registry::RegistryClient,
    util,
};
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};
use syneroym_sdk::SyneroymClient;
use syneroym_substrate::identity;
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{mpsc, mpsc::Sender},
    task::JoinHandle,
    time,
};

#[derive(Clone, Copy)]
struct PortBlock {
    supervisor_iroh: u16,
    supervisor_registry: u16,
    supervisor_gateway: u16,
    managed_iroh: u16,
    managed_registry: u16,
    managed_gateway: u16,
}

// The next free block after `topology_document_e2e.rs`'s 15_400-16_502,
// the highest claimed at the time this file was written -- one block of
// six per test, since cargo runs every test in this binary concurrently.
const PORTS_HOSTNAME_ALONE: PortBlock = PortBlock {
    supervisor_iroh: 16_600,
    supervisor_registry: 16_601,
    supervisor_gateway: 16_602,
    managed_iroh: 16_700,
    managed_registry: 16_701,
    managed_gateway: 16_702,
};
const PORTS_ROUTING_KEY: PortBlock = PortBlock {
    supervisor_iroh: 16_800,
    supervisor_registry: 16_801,
    supervisor_gateway: 16_802,
    managed_iroh: 16_900,
    managed_registry: 16_901,
    managed_gateway: 16_902,
};
const PORTS_NO_GRANT_REFUSED: PortBlock = PortBlock {
    supervisor_iroh: 17_000,
    supervisor_registry: 17_001,
    supervisor_gateway: 17_002,
    managed_iroh: 17_100,
    managed_registry: 17_101,
    managed_gateway: 17_102,
};
const PORTS_SAME_NODE_NO_CREDENTIAL: PortBlock = PortBlock {
    supervisor_iroh: 17_200,
    supervisor_registry: 17_201,
    supervisor_gateway: 17_202,
    managed_iroh: 17_300,
    managed_registry: 17_301,
    managed_gateway: 17_302,
};

const MANAGED_ALIAS: &str = "managed";

/// A full, independently-identified `syneroym-substrate` instance.
struct Node {
    registry_url: String,
    gateway_port: u16,
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
        grant_resolve_to_node_did: bool,
        resolve_ucan: Option<&CapabilityToken>,
        identity_key_path: Option<PathBuf>,
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
        // A caller-supplied key file makes this node's own DID
        // deterministic and known before boot -- needed to mint a
        // resolve grant naming this node's DID as grantee. `None` keeps
        // today's behaviour: `setup_substrate_identity` generates and
        // persists a fresh key under `app_data_dir`.
        config.identity.key = identity_key_path;

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

        let resolve_ucan_path = resolve_ucan.map(|token| {
            let path = base_path.join("resolve_ucan.json");
            let mut f = std::fs::File::create(&path).expect("create resolve_ucan file");
            f.write_all(serde_json::to_string(token).unwrap().as_bytes())
                .expect("write resolve_ucan file");
            path
        });
        config.roles.client_gateway =
            Some(ClientGatewayRole { http_port: gateway_port, resolve_ucan: resolve_ucan_path });
        config.iam.admin_ucan_root = Some(substrate::derive_did_key(&owner.public_key()));
        config.iam.grant_resolve_to_node_did = grant_resolve_to_node_did;
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
            gateway_port,
            substrate_client,
            shutdown_tx,
            substrate_handle,
            _temp_dir: temp_dir,
        }
    }

    fn did(&self) -> &str {
        self.substrate_client.service_id()
    }

    fn gateway_url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.gateway_port)
    }

    async fn teardown(mut self) {
        let _ = self.substrate_client.shutdown().await;
        let _ = self.shutdown_tx.send(()).await;
        let _ = self.substrate_handle.await;
    }
}

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

/// A bare `substrate:<supervisor_node_did>` capability with ability
/// `supervisor/resolve` (D-S2-7's shape), issued to `grantee_did` by the
/// supervisor node's own owner. Bare scope short-circuits
/// `Capability::grants`, so it covers `synapp:<any-app-did>` supervised on
/// that node -- which is what lets this be minted right after the
/// supervisor node boots, before any app exists, exactly the operator
/// grant `[roles.client_gateway] resolve_ucan` is meant to hold.
fn resolve_grant_for_node(
    supervisor_owner: &Identity,
    grantee_did: &str,
    supervisor_node_did: &str,
) -> CapabilityToken {
    CapabilityToken::issue(
        supervisor_owner,
        grantee_did,
        vec![Capability {
            with: ResourceUri::substrate(supervisor_node_did),
            can: Ability(Ability::SUPERVISOR_RESOLVE.to_string()),
            caveats: None,
        }],
        Map::new(),
        3600,
        vec![],
    )
    .expect("issue resolve grant")
}

/// `replicas`-member manifest, `backend` placed on `MANAGED_ALIAS`, its
/// TCP source pointing at `backend_port` -- a real listener this file
/// spawns, so a request routed through it carries real, checkable bytes.
fn service_manifest(replicas: u32, backend_port: u16) -> SynAppManifest {
    let mut services = BTreeMap::new();
    services.insert(
        LogicalServiceName::new("backend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: format!("127.0.0.1:{backend_port}"),
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
        id: AppBlueprintId::new("syneroym:gateway-hostname-test-app"),
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

/// Submits and adopts a `replicas`-member instance whose `backend` TCP
/// source is `backend_port`, returning the app master DID, and waits for
/// the Tier-1 record to resolve through the registry (published by the
/// resident loop's own tick, not synchronously by `adopt`).
async fn submit_and_adopt(
    supervisor_node: &Node,
    instance_id: &str,
    inventory_json: String,
    replicas: u32,
    backend_port: u16,
) -> String {
    let manifest = service_manifest(replicas, backend_port);
    let plan_json = compiled_plan_json(&manifest, instance_id).await;
    supervisor_node
        .substrate_client
        .request("supervisor", "submit", submission(instance_id, plan_json, inventory_json, 0))
        .await
        .expect("submit failed");
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

/// Spawns a minimal raw-TCP responder: whatever bytes arrive, it answers
/// with one fixed HTTP response carrying `marker`, and the raw
/// `X-Syneroym-Routing-Key` header value it received (or `"none"` if the
/// request carried none), in the body. `ServiceType::Tcp` proxies raw
/// bytes end to end (the client gateway's own HTTP parsing only ever
/// reads the `Host` header before forwarding the request verbatim), so
/// this is a faithful stand-in for a real backend without pulling in a
/// full HTTP server crate. Echoing the routing-key value back is what
/// makes test 100 able to fail: every replica of a TCP service in this
/// milestone shares one physical backend by construction
/// (`service_manifest` clones the same `config.source` per `member_index`
/// -- `compiler.rs`'s `PlannedService` loop -- so response *content*
/// cannot distinguish which member DID the gateway dialed). What crossing
/// the real wire intact end to end can prove instead, and what this
/// test's own doc comment already promises, is that the header itself
/// survives the gateway unmodified -- this is the check that lets it.
async fn spawn_tcp_backend(port: u16, marker: &'static str) -> JoinHandle<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await.expect("bind tcp backend");
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { break };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let routing_key =
                    extract_header_value(&buf[..n], "x-syneroym-routing-key").unwrap_or_default();
                let routing_key = if routing_key.is_empty() { "none" } else { &routing_key };
                let body = format!("Hello from {marker}; routing-key={routing_key}");
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes()).await;
                let _ = stream.flush().await;
            });
        }
    })
}

/// A minimal, case-insensitive raw-HTTP header value extractor over the
/// exact bytes `spawn_tcp_backend` reads off the wire -- deliberately not
/// pulling in a full HTTP parser for a header lookup this simple.
fn extract_header_value(raw: &[u8], header_name: &str) -> Option<String> {
    let text = String::from_utf8_lossy(raw);
    let needle = format!("{}:", header_name.to_ascii_lowercase());
    text.lines().find_map(|line| {
        let lower = line.to_ascii_lowercase();
        lower.starts_with(&needle).then(|| line[needle.len()..].trim().to_string())
    })
}

async fn poll_gateway_until_success(
    client: &Client,
    gateway_url: &str,
    host: &str,
    routing_key: Option<&str>,
    deadline: Instant,
) -> String {
    loop {
        let mut req = client.post(gateway_url).header("Host", host).body("ping");
        if let Some(key) = routing_key {
            req = req.header("X-Syneroym-Routing-Key", key);
        }
        match req.send().await {
            Ok(r) if r.status().is_success() => return r.text().await.unwrap(),
            _ if Instant::now() < deadline => time::sleep(Duration::from_millis(300)).await,
            Ok(r) => panic!("gateway request to '{host}' failed with status {}", r.status()),
            Err(e) => panic!("gateway request to '{host}' failed: {e}"),
        }
    }
}

/// Boots a supervisor node and a managed node (B sharing A's
/// registry/relay), grants the supervisor's own node-wide `orchestrator/
/// deploy` on the managed node, and returns everything a test needs to
/// call `submit`, plus the supervisor's own node DID (for minting a
/// resolve grant once the caller's own DID is known).
#[allow(clippy::too_many_arguments)]
async fn boot_pair(
    supervisor_owner: &Identity,
    managed_owner: &Identity,
    ports: PortBlock,
    supervisor_grant_resolve_to_node_did: bool,
    supervisor_resolve_ucan: Option<&CapabilityToken>,
    managed_grant_resolve_to_node_did: bool,
    managed_resolve_ucan: Option<&CapabilityToken>,
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
        supervisor_grant_resolve_to_node_did,
        supervisor_resolve_ucan,
        None,
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
        managed_grant_resolve_to_node_did,
        managed_resolve_ucan,
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

/// Test 99: two real substrates and a real registry -- submit + adopt an
/// app with `replicas > 1`, build the host with `generate_app_host`, POST
/// through the gateway, assert the app answered. The milestone's
/// cross-app half, from an ordinary HTTP client. The managed node's own
/// gateway (a different node from the one supervising the app) resolves
/// it via an operator-supplied `resolve_ucan`.
#[tokio::test]
async fn an_http_client_reaches_an_apps_logical_service_by_hostname_alone() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let backend_port = 42_600u16;
    let _backend = spawn_tcp_backend(backend_port, "hostname-alone").await;

    // The managed node's own DID (its client gateway's caller identity,
    // D-S3-6) must be known before the resolve grant naming it as
    // grantee can be minted -- derived up front from a key file, then
    // handed to `Node::boot` so the booted node's identity matches it.
    let managed_key_dir = tempfile::tempdir().unwrap();
    let managed_identity = Identity::generate().unwrap();
    managed_identity.save_to_path(managed_key_dir.path().join("substrate.key")).unwrap();
    let managed_node_did = substrate::derive_did_key(&managed_identity.public_key());

    // The supervisor node's own DID is needed to name the grant's bare
    // `substrate:` resource, so it must boot first.
    let _ = ring::default_provider().install_default();
    let supervisor_node = Node::boot(
        PORTS_HOSTNAME_ALONE.supervisor_iroh,
        PORTS_HOSTNAME_ALONE.supervisor_registry,
        PORTS_HOSTNAME_ALONE.supervisor_gateway,
        None,
        None,
        &supervisor_owner,
        Some(supervisor_role()),
        false,
        None,
        None,
    )
    .await;
    let resolve_token =
        resolve_grant_for_node(&supervisor_owner, &managed_node_did, supervisor_node.did());

    let shared_registry = supervisor_node.registry_url.clone();
    let shared_relay = format!("http://localhost:{}", PORTS_HOSTNAME_ALONE.supervisor_iroh);
    let managed_node = Node::boot(
        PORTS_HOSTNAME_ALONE.managed_iroh,
        PORTS_HOSTNAME_ALONE.managed_registry,
        PORTS_HOSTNAME_ALONE.managed_gateway,
        Some(shared_registry),
        Some(shared_relay),
        &managed_owner,
        None,
        false,
        Some(&resolve_token),
        Some(managed_key_dir.path().join("substrate.key")),
    )
    .await;
    assert_eq!(managed_node.did(), managed_node_did, "the pre-derived DID must match the boot");

    let grant =
        node_wide_supervisor_grant(&managed_owner, supervisor_node.did(), managed_node.did());
    let inventory_json = serde_json::to_string(&BTreeMap::from([(
        MANAGED_ALIAS.to_string(),
        SupervisorInventoryEntry {
            did: managed_node.did().to_string(),
            api_url: Some(managed_node.registry_url.clone()),
            ucan: Some(grant),
        },
    )]))
    .unwrap();

    let app_did =
        submit_and_adopt(&supervisor_node, "gw-host-alone", inventory_json, 2, backend_port).await;
    let host =
        util::generate_app_host("gw-host-alone", &app_did, "backend", None, "localhost").unwrap();

    let text = poll_gateway_until_success(
        &Client::new(),
        &managed_node.gateway_url(),
        &host,
        None,
        Instant::now() + Duration::from_secs(20),
    )
    .await;
    assert!(text.contains("hostname-alone"), "{text}");

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

/// Test 100: the routing-key header over the wire, against a real
/// `Redundant` service. **Corrected post-review-pass (finding C1)**: the
/// original version asserted `keyed_first == keyed_second` over identical
/// response *content*, which was true regardless of correctness -- every
/// replica of a TCP service shares one physical backend by construction
/// in this milestone (`service_manifest` clones the same `config.source`
/// per `member_index`), so content can never distinguish which member the
/// gateway dialed, and it reused one `Client` across every request, so a
/// gateway bug that only reads the header on a connection's first request
/// (the exact shape finding A2 fixed) would have gone unnoticed too.
///
/// What this test actually verifies now, each over its own fresh
/// connection (a reused one would prove nothing about a *second*
/// request): the header's exact bytes reach the backend unmodified,
/// per request, and an absent header reaches it as absent rather than as
/// some stale value from an earlier request. Per-member selection
/// consistency (the same key always picks the same member) is unit
/// tested directly, with real members to distinguish, at `syneroym-sdk`
/// test 88.
#[tokio::test]
async fn a_routing_key_header_crosses_the_gateway_to_the_backend_per_request() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let backend_port = 42_610u16;
    let _backend = spawn_tcp_backend(backend_port, "keyed").await;

    let (supervisor_node, managed_node, inventory_json) = boot_pair(
        &supervisor_owner,
        &managed_owner,
        PORTS_ROUTING_KEY,
        true, // same-node gate: the supervisor's own gateway resolves its own app
        None,
        false,
        None,
    )
    .await;

    let app_did =
        submit_and_adopt(&supervisor_node, "gw-host-keyed", inventory_json, 2, backend_port).await;
    let host =
        util::generate_app_host("gw-host-keyed", &app_did, "backend", None, "localhost").unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);

    let unkeyed = poll_gateway_until_success(
        &Client::new(),
        &supervisor_node.gateway_url(),
        &host,
        None,
        deadline,
    )
    .await;
    assert!(unkeyed.contains("routing-key=none"), "{unkeyed}");

    let keyed_alice = poll_gateway_until_success(
        &Client::new(),
        &supervisor_node.gateway_url(),
        &host,
        Some("alice"),
        deadline,
    )
    .await;
    assert!(keyed_alice.contains("routing-key=alice"), "{keyed_alice}");

    // A different key on a third, again fresh, connection -- proving the
    // value genuinely travels with each request rather than being fixed
    // once (by a stale cache, or a gateway bug reading only the first
    // connection's headers) and echoed back regardless of what is sent.
    let keyed_bob = poll_gateway_until_success(
        &Client::new(),
        &supervisor_node.gateway_url(),
        &host,
        Some("bob"),
        deadline,
    )
    .await;
    assert!(keyed_bob.contains("routing-key=bob"), "{keyed_bob}");

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

/// Test 101: matrix row 7 at the hostname layer -- a clean 502 with the
/// denial logged, and no member DID anywhere in the response. The gateway
/// node has the same-node gate **off** and no `resolve_ucan`, against an
/// app supervised elsewhere -- the exact shape D-S3-6 says needs a token.
#[tokio::test]
async fn an_app_scoped_hostname_for_an_app_this_gateway_holds_no_grant_for_is_refused() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let backend_port = 42_620u16;
    let _backend = spawn_tcp_backend(backend_port, "no-grant").await;

    let (supervisor_node, managed_node, inventory_json) = boot_pair(
        &supervisor_owner,
        &managed_owner,
        PORTS_NO_GRANT_REFUSED,
        false,
        None,
        false, // no same-node gate
        None,  // no resolve_ucan
    )
    .await;

    let app_did =
        submit_and_adopt(&supervisor_node, "gw-host-no-grant", inventory_json, 1, backend_port)
            .await;
    let host = util::generate_app_host("gw-host-no-grant", &app_did, "backend", None, "localhost")
        .unwrap();

    // The managed node's own gateway has no standing on the supervisor's
    // app at all.
    let res = Client::new()
        .post(managed_node.gateway_url())
        .header("Host", &host)
        .body("ping")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 502);
    let body = res.text().await.unwrap();
    assert!(!body.contains("did:key"), "must carry no member DID: {body}");

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

/// Test 102: D-S3-6(a) end to end -- one substrate running both the
/// supervisor and the gateway, `[iam].grant_resolve_to_node_did = true`,
/// no `resolve_ucan` anywhere. The single-node developer path, and the
/// case most likely to regress silently, since every other e2e in this
/// file hands out an explicit `resolve_ucan` or scopes to the pair's own
/// same-node grant deliberately.
#[tokio::test]
async fn a_same_node_gateway_resolves_with_no_credential_file() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let backend_port = 42_630u16;
    let _backend = spawn_tcp_backend(backend_port, "same-node").await;

    let (supervisor_node, managed_node, inventory_json) = boot_pair(
        &supervisor_owner,
        &managed_owner,
        PORTS_SAME_NODE_NO_CREDENTIAL,
        true, // the supervisor node's own gateway gets the same-node grant
        None, // no resolve_ucan anywhere
        false,
        None,
    )
    .await;

    let app_did =
        submit_and_adopt(&supervisor_node, "gw-host-same-node", inventory_json, 1, backend_port)
            .await;
    let host = util::generate_app_host("gw-host-same-node", &app_did, "backend", None, "localhost")
        .unwrap();

    // The supervisor node's own gateway resolves its own app, no
    // credential file involved -- the bare `substrate:<node_did>` grant
    // covers it because the caller DID (the gateway's own node identity)
    // equals the node this app is supervised on.
    let text = poll_gateway_until_success(
        &Client::new(),
        &supervisor_node.gateway_url(),
        &host,
        None,
        Instant::now() + Duration::from_secs(20),
    )
    .await;
    assert!(text.contains("same-node"), "{text}");

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}
