#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The logical discovery overlay (ADR-0022 §7): the gateway hostname scheme
//! and the routing-key header, proven end to end against two real substrates
//! and a real registry -- extended past the SDK/RPC layer to an ordinary
//! HTTP client addressing an app purely by hostname.
//!
//! The supervisor/managed pair and the submit helpers come from `common`.
//! This file adds one credential lever on top: the resolve grant is minted
//! as a bare `substrate:<supervisor-node-did>` capability (the same shape
//! the same-node gate uses) rather than scoped to one `synapp:<app-did>`,
//! so it can be minted right after the supervisor node boots, before any
//! app exists, instead of needing a second boot pass once `adopt` reveals
//! the app DID.

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    time::{Duration, Instant},
};

use common::SubstrateNode;
use reqwest::Client;
use rustls::crypto::ring;
use semver::Version;
use serde_json::{Map, json};
use syneroym_app_orchestration::{
    LogicalServiceName, TopologyVisibility, Visibility,
    models::{
        AppBlueprintId, PlacementSelector, ServiceConfig, ServiceSpec, ServiceType, SubstrateAlias,
        SynAppManifest,
    },
};
use syneroym_core::{dht_registry::RegistryClient, util};
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time,
};

mod common;

const MANAGED_ALIAS: &str = "managed";

/// A bare `substrate:<supervisor_node_did>` capability with ability
/// `supervisor/resolve`, issued to `grantee_did` by the supervisor node's
/// own owner. Bare scope short-circuits `Capability::grants`, so it covers
/// `synapp:<any-app-did>` supervised on that node -- which is what lets this
/// be minted right after the supervisor node boots, before any app exists,
/// exactly the operator grant `[roles.client_gateway] resolve_ucan` is
/// meant to hold.
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

fn service_manifest_with_vis(
    replicas: u32,
    backend_port: u16,
    visibility: Visibility,
    topology_visibility: TopologyVisibility,
) -> SynAppManifest {
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
        id: AppBlueprintId::new("syneroym:gateway-hostname-test-app"),
        version: Version::new(0, 1, 0),
        description: None,
        placement: None,
        services,
        dependencies: BTreeMap::new(),
    }
}

/// `replicas`-member manifest, `backend` placed on `MANAGED_ALIAS`, its
/// TCP source pointing at `backend_port` -- a real listener this file
/// spawns, so a request routed through it carries real, checkable bytes.
fn service_manifest(replicas: u32, backend_port: u16) -> SynAppManifest {
    service_manifest_with_vis(
        replicas,
        backend_port,
        Visibility::Internal,
        TopologyVisibility::Restricted,
    )
}

async fn submit_and_adopt_with_manifest(
    supervisor_node: &SubstrateNode,
    instance_id: &str,
    inventory_json: String,
    manifest: &SynAppManifest,
) -> String {
    let plan_json = common::compiled_plan_json(manifest, instance_id).await;
    supervisor_node
        .substrate_client
        .request(
            "supervisor",
            "submit",
            common::submission(instance_id, plan_json, inventory_json, 0),
        )
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

/// Submits and adopts a `replicas`-member instance whose `backend` TCP
/// source is `backend_port`, returning the app master DID, and waits for
/// the Tier-1 record to resolve through the registry (published by the
/// resident loop's own tick, not synchronously by `adopt`).
async fn submit_and_adopt(
    supervisor_node: &SubstrateNode,
    instance_id: &str,
    inventory_json: String,
    replicas: u32,
    backend_port: u16,
) -> String {
    let manifest = service_manifest(replicas, backend_port);
    submit_and_adopt_with_manifest(supervisor_node, instance_id, inventory_json, &manifest).await
}

/// Spawns a minimal raw-TCP responder: whatever bytes arrive, it answers
/// with one fixed HTTP response carrying `marker`, and the raw
/// `X-Syneroym-Routing-Key` header value it received (or `"none"` if the
/// request carried none), in the body. `ServiceType::Tcp` proxies raw
/// bytes end to end (the client gateway's own HTTP parsing only ever
/// reads the `Host` header before forwarding the request verbatim), so
/// this is a faithful stand-in for a real backend without pulling in a
/// full HTTP server crate. Echoing the routing-key value back is what
/// lets the routing-key test fail on a regression: every replica of a TCP
/// service shares one physical backend by construction here
/// (`service_manifest` clones the same `config.source` per member), so
/// response *content* cannot distinguish which member DID the gateway
/// dialed. What crossing the real wire intact end to end can prove
/// instead is that the header itself survives the gateway unmodified.
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

/// [`SubstrateNode::gateway_url`] with the trailing slash this file's
/// gateway requests are built against.
fn gateway_url(node: &SubstrateNode) -> String {
    format!("{}/", node.gateway_url())
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

/// Boots a supervisor node and a managed node (sharing the supervisor's
/// registry and relay), grants the supervisor its own node-wide
/// `orchestrator/deploy` on the managed node, and returns everything a test
/// needs to call `submit`. `supervisor_grant_resolve_to_node_did` toggles
/// `[iam].grant_resolve_to_node_did` on the supervisor node -- the
/// single-node developer path where a gateway resolves its own node's apps
/// with no credential file.
async fn boot_pair(
    supervisor_owner: &Identity,
    managed_owner: &Identity,
    supervisor_grant_resolve_to_node_did: bool,
) -> (SubstrateNode, SubstrateNode, String) {
    let _ = ring::default_provider().install_default();

    let supervisor_node = SubstrateNode::builder()
        .owner(supervisor_owner)
        .supervisor(common::supervisor_role(2))
        .inject_kek()
        .configure(move |config| {
            config.iam.grant_resolve_to_node_did = supervisor_grant_resolve_to_node_did;
        })
        .boot()
        .await;

    let managed_node = SubstrateNode::builder()
        .owner(managed_owner)
        .shared_registry(supervisor_node.registry_url())
        .shared_relay(supervisor_node.relay_url())
        .inject_kek()
        .boot()
        .await;

    let grant = common::node_wide_supervisor_grant(
        managed_owner,
        supervisor_node.did(),
        managed_node.did(),
    );
    let inventory_json = common::inventory_json(MANAGED_ALIAS, &managed_node, grant);
    (supervisor_node, managed_node, inventory_json)
}

/// Two real substrates and a real registry -- submit + adopt an app with
/// `replicas > 1`, POST through the gateway, assert the app answered, from
/// an ordinary HTTP client. The managed node's own gateway (a different
/// node from the one supervising the app) resolves it via an
/// operator-supplied `resolve_ucan`.
#[tokio::test]
async fn an_http_client_reaches_an_apps_logical_service_by_hostname_alone() {
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let [backend_port] = common::alloc_ports();
    let _backend = spawn_tcp_backend(backend_port, "hostname-alone").await;

    // The managed node's own DID (its client gateway's caller identity)
    // must be known before the resolve grant naming it as grantee can be
    // minted -- derived up front from a key file, then handed to the
    // builder so the booted node's identity matches it.
    let managed_key_dir = tempfile::tempdir().unwrap();
    let managed_identity = Identity::generate().unwrap();
    let managed_key_path = managed_key_dir.path().join("substrate.key");
    managed_identity.save_to_path(&managed_key_path).unwrap();
    let managed_node_did = substrate::derive_did_key(&managed_identity.public_key());

    // The supervisor node's own DID is needed to name the grant's bare
    // `substrate:` resource, so it must boot first.
    let _ = ring::default_provider().install_default();
    let supervisor_node = SubstrateNode::builder()
        .owner(&supervisor_owner)
        .supervisor(common::supervisor_role(2))
        .inject_kek()
        .boot()
        .await;
    let resolve_token =
        resolve_grant_for_node(&supervisor_owner, &managed_node_did, supervisor_node.did());
    let resolve_ucan_path = managed_key_dir.path().join("resolve_ucan.json");
    std::fs::write(&resolve_ucan_path, serde_json::to_string(&resolve_token).unwrap()).unwrap();

    let managed_node = SubstrateNode::builder()
        .owner(&managed_owner)
        .shared_registry(supervisor_node.registry_url())
        .shared_relay(supervisor_node.relay_url())
        .inject_kek()
        .configure(move |config| {
            config.identity.key = Some(managed_key_path.clone());
            config.roles.client_gateway.as_mut().expect("client gateway role").resolve_ucan =
                Some(resolve_ucan_path.clone());
        })
        .boot()
        .await;
    assert_eq!(managed_node.did(), managed_node_did, "the pre-derived DID must match the boot");

    let grant = common::node_wide_supervisor_grant(
        &managed_owner,
        supervisor_node.did(),
        managed_node.did(),
    );
    let inventory_json = common::inventory_json(MANAGED_ALIAS, &managed_node, grant);

    let app_did =
        submit_and_adopt(&supervisor_node, "gw-host-alone", inventory_json, 2, backend_port).await;
    let host =
        util::generate_app_host("gw-host-alone", &app_did, "backend", None, "localhost").unwrap();

    let managed_gateway = gateway_url(&managed_node);
    let text = poll_gateway_until_success(
        &Client::new(),
        &managed_gateway,
        &host,
        None,
        Instant::now() + Duration::from_secs(20),
    )
    .await;
    assert!(text.contains("hostname-alone"), "{text}");

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

/// The routing-key header over the wire, against a real `Redundant`
/// service, each request over its own fresh connection (a reused one would
/// prove nothing about a *second* request): the header's exact bytes reach
/// the backend unmodified, per request, and an absent header reaches it as
/// absent rather than as some stale value from an earlier request.
/// Per-member selection consistency (the same key always picks the same
/// member) is unit tested directly, with real members to distinguish, in
/// `syneroym-sdk`.
#[tokio::test]
async fn a_routing_key_header_crosses_the_gateway_to_the_backend_per_request() {
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let [backend_port] = common::alloc_ports();
    let _backend = spawn_tcp_backend(backend_port, "keyed").await;

    let (supervisor_node, managed_node, inventory_json) = boot_pair(
        &supervisor_owner,
        &managed_owner,
        true, // same-node gate: the supervisor's own gateway resolves its own app
    )
    .await;

    let app_did =
        submit_and_adopt(&supervisor_node, "gw-host-keyed", inventory_json, 2, backend_port).await;
    let host =
        util::generate_app_host("gw-host-keyed", &app_did, "backend", None, "localhost").unwrap();

    let supervisor_gateway = gateway_url(&supervisor_node);
    let deadline = Instant::now() + Duration::from_secs(20);

    let unkeyed =
        poll_gateway_until_success(&Client::new(), &supervisor_gateway, &host, None, deadline)
            .await;
    assert!(unkeyed.contains("routing-key=none"), "{unkeyed}");

    let keyed_alice = poll_gateway_until_success(
        &Client::new(),
        &supervisor_gateway,
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
        &supervisor_gateway,
        &host,
        Some("bob"),
        deadline,
    )
    .await;
    assert!(keyed_bob.contains("routing-key=bob"), "{keyed_bob}");

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

/// A clean 502 with the denial logged, and no member DID anywhere in the
/// response. The gateway node has the same-node gate **off** and no
/// `resolve_ucan`, against an app supervised elsewhere -- the exact shape
/// that needs a token.
#[tokio::test]
async fn an_app_scoped_hostname_for_an_app_this_gateway_holds_no_grant_for_is_refused() {
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let [backend_port] = common::alloc_ports();
    let _backend = spawn_tcp_backend(backend_port, "no-grant").await;

    let (supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, false).await;

    let app_did =
        submit_and_adopt(&supervisor_node, "gw-host-no-grant", inventory_json, 1, backend_port)
            .await;
    let host = util::generate_app_host("gw-host-no-grant", &app_did, "backend", None, "localhost")
        .unwrap();

    // The managed node's own gateway has no standing on the supervisor's
    // app at all.
    let res = Client::new()
        .post(gateway_url(&managed_node))
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

/// One substrate running both the supervisor and the gateway,
/// `[iam].grant_resolve_to_node_did = true`, no `resolve_ucan` anywhere.
/// The single-node developer path, and the case most likely to regress
/// silently, since every other e2e in this file hands out an explicit
/// `resolve_ucan` or scopes to the pair's own same-node grant deliberately.
#[tokio::test]
async fn a_same_node_gateway_resolves_with_no_credential_file() {
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let [backend_port] = common::alloc_ports();
    let _backend = spawn_tcp_backend(backend_port, "same-node").await;

    let (supervisor_node, managed_node, inventory_json) = boot_pair(
        &supervisor_owner,
        &managed_owner,
        true, // the supervisor node's own gateway gets the same-node grant
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
    let supervisor_gateway = gateway_url(&supervisor_node);
    let text = poll_gateway_until_success(
        &Client::new(),
        &supervisor_gateway,
        &host,
        None,
        Instant::now() + Duration::from_secs(20),
    )
    .await;
    assert!(text.contains("same-node"), "{text}");

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

/// A client gateway with neither credential resolves and proxies an `open`
/// logical hostname.
#[tokio::test]
async fn a_gateway_with_neither_credential_resolves_and_proxies_an_open_logical_hostname() {
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let [backend_port] = common::alloc_ports();
    let _backend = spawn_tcp_backend(backend_port, "open-hello").await;

    let (supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, false).await;

    let manifest =
        service_manifest_with_vis(1, backend_port, Visibility::Internal, TopologyVisibility::Open);
    let app_did =
        submit_and_adopt_with_manifest(&supervisor_node, "gw-host-open", inventory_json, &manifest)
            .await;
    let host =
        util::generate_app_host("gw-host-open", &app_did, "backend", None, "localhost").unwrap();

    let managed_gateway = gateway_url(&managed_node);
    let text = poll_gateway_until_success(
        &Client::new(),
        &managed_gateway,
        &host,
        None,
        Instant::now() + Duration::from_secs(20),
    )
    .await;
    assert!(text.contains("open-hello"), "{text}");

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

/// The negative half: a client gateway with neither credential refuses a
/// `restricted` logical hostname.
#[tokio::test]
async fn a_gateway_with_neither_credential_refuses_a_restricted_logical_hostname() {
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let [backend_port] = common::alloc_ports();
    let _backend = spawn_tcp_backend(backend_port, "restricted-hello").await;

    let (supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, false).await;

    let manifest = service_manifest_with_vis(
        1,
        backend_port,
        Visibility::Internal,
        TopologyVisibility::Restricted,
    );
    let app_did = submit_and_adopt_with_manifest(
        &supervisor_node,
        "gw-host-restricted",
        inventory_json,
        &manifest,
    )
    .await;
    let host =
        util::generate_app_host("gw-host-restricted", &app_did, "backend", None, "localhost")
            .unwrap();

    let res = Client::new()
        .post(gateway_url(&managed_node))
        .header("Host", &host)
        .body("ping")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 502);

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}
