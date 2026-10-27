//! Integration tests for the multi-hop relay functionality
//!
//! Simulates a scenario that organizes relays, registries, and substrates
//! across network boundaries in a hierarchy and tests bidirectional e2e
//! connectivity between clients and substrates across networks.
#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::{path::Path, time::Duration};

use anyhow::Result;
use iroh::{Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl, SecretKey, protocol::Router};
use reqwest::Client;
use syneroym_community_registry::EcosystemRegistry;
use syneroym_core::{
    config::{ServiceRegistryRole, SubstrateConfig},
    dht_registry::{
        EndpointInfo, EndpointMechanism, EndpointType, RegistryClient, SignedEndpointInfo,
    },
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    test_constants,
};
use syneroym_data_db::registry_store;
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_router::{
    AdaptationStage, EncryptionStage, RouteHandler, RoutePipeline, RoutePreamble, RouteProtocol,
    RouteTransport, SYNEROYM_ALPN, ServiceStage, TransportStage, net_iroh,
};
use syneroym_sdk::SyneroymClient;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};
use tokio::time;

mod common;
use common::{
    logs::{forwarded_without_identity, init_trace_collector},
    nodes::{
        InboundRelayTopology, TargetSubstrate, build_test_route_handler_deps,
        create_signed_info_with_full_addr, spawn_global_coordinator, spawn_private_coordinator,
        wait_for_local_addr,
    },
};

/// Minimal `DeployManifest` for `AppSandboxEngine::deploy_wasm` (the
/// cross-node proxy test) -- no `custom_config`/quota, matching
/// `lifecycle_hooks.rs`'s own `wasm_deploy_manifest` helper.
fn wasm_deploy_manifest(bytes: Vec<u8>) -> DeployManifest {
    DeployManifest::unpublished(
        ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: None,
            visibility: None,
        },
        ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(bytes),
            hash: None,
            interfaces: vec![],
        }),
    )
}

fn create_signed_info(
    identity: &Identity,
    service_id: &str,
    endpoint_addr: &EndpointAddr,
    relay_url: Option<String>,
) -> SignedEndpointInfo {
    let pruned_addr = EndpointAddr::new(endpoint_addr.id);
    let endpoint_addr_bytes = serde_json::to_vec(&pruned_addr).unwrap();
    let info = EndpointInfo {
        service_id: service_id.to_string(),
        substrate_id: service_id.to_string(),
        endpoint_type: EndpointType::Substrate,
        nickname: Some("test-node".to_string()),
        mechanisms: vec![EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url }],
        is_private: false,
        ttl: None,
        not_after: u64::MAX / 2,
        generation: 0,
        mailbox: None,
    };

    info.sign(identity).unwrap()
}

/// Spawns target substrate Sx in the public network under C, exactly as
/// `test_outbound_relay` does: bound directly to C's relay, then registered
/// in registry R.
async fn spawn_x_target_substrate(
    base_path: &Path,
    r_url: &str,
    c_relay_url: &str,
    info_client: &Client,
) -> Result<TargetSubstrate> {
    let identity_x = Identity::generate()?;
    let secret_x_bytes = identity_x.to_bytes();
    let did_x = derive_did_key(&identity_x.public_key());

    let mut config_x = SubstrateConfig {
        app_local_data_dir: base_path.join("data_x"),
        app_data_dir: base_path.join("user_data_x"),
        ..Default::default()
    };
    config_x.substrate.enable_bep0044_dht = false;
    config_x.resolve_paths();
    let data_store_x = registry_store::init_store(&config_x).await?;
    let endpoint_registry_x = EndpointRegistry::new(data_store_x).await?;

    let endpoint_x = SubstrateEndpoint::NativeHostChannel { service_id: did_x.clone() };
    endpoint_registry_x.register(did_x.clone(), "orchestrator".to_string(), endpoint_x).await?;

    // Bind Sx to Iroh so C can connect to it (Sx uses C's relay url). Built
    // *before* `RouteHandler::init` -- see the matching comment on Sz above.
    let mut ep_x_bldr = net_iroh::empty_endpoint_builder();
    ep_x_bldr = ep_x_bldr
        .relay_mode(RelayMode::Custom(RelayMap::from(c_relay_url.parse::<RelayUrl>().unwrap())));
    let secret_key_x = SecretKey::from_bytes(&secret_x_bytes);
    let ep_x = ep_x_bldr.secret_key(secret_key_x).bind().await?;
    ep_x.online().await;

    let deps_x = build_test_route_handler_deps(&config_x, &did_x, &endpoint_registry_x).await?;
    let route_handler_x = RouteHandler::init(
        did_x.clone(),
        &config_x,
        endpoint_registry_x,
        secret_x_bytes,
        Some(ep_x.clone()),
        deps_x,
    )
    .await?;

    let ep_x_addr = ep_x.addr();
    let router_x = Router::builder(ep_x).accept(SYNEROYM_ALPN, route_handler_x.clone()).spawn();

    // Register Sx in community registry R
    let signed_info_x =
        create_signed_info(&identity_x, &did_x, &ep_x_addr, Some(c_relay_url.to_string()));
    let res = info_client.post(format!("{r_url}/register")).json(&signed_info_x).send().await?;
    assert!(res.status().is_success());

    Ok(TargetSubstrate { did: did_x, router: router_x, route_handler: route_handler_x })
}

#[tokio::test]
async fn test_registry_propagation() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();

    // 1. Spawn global registry R
    let mut config_r = SubstrateConfig {
        app_local_data_dir: base_path.join("data_r"),
        app_data_dir: base_path.join("user_data_r"),
        ..Default::default()
    };
    config_r.substrate.enable_bep0044_dht = false;
    config_r.roles.community_registry = Some(ServiceRegistryRole {
        http_bind_address: "127.0.0.1:0".to_string(),
        ..Default::default()
    });
    let mut registry_r = EcosystemRegistry::init(&config_r).await?;
    let r_url = registry_r.bind().await?;
    registry_r.spawn().await?;

    // 2. Spawn private registry Rp pointing to R
    let mut config_rp = SubstrateConfig {
        app_local_data_dir: base_path.join("data_rp"),
        app_data_dir: base_path.join("user_data_rp"),
        ..Default::default()
    };
    config_rp.substrate.enable_bep0044_dht = false;
    config_rp.roles.community_registry = Some(ServiceRegistryRole {
        http_bind_address: "127.0.0.1:0".to_string(),
        parent_registry_url: Some(r_url.clone()),
        ..Default::default()
    });
    let mut registry_rp = EcosystemRegistry::init(&config_rp).await?;
    let rp_url = registry_rp.bind().await?;
    registry_rp.spawn().await?;

    // 3. Register info on Rp
    let identity = Identity::generate()?;
    let did = derive_did_key(&identity.public_key());
    let dummy_secret = SecretKey::generate();
    let dummy_ep = net_iroh::empty_endpoint_builder().secret_key(dummy_secret).bind().await?;
    let dummy_addr = dummy_ep.addr();

    let signed_info = create_signed_info(&identity, &did, &dummy_addr, None);

    let client = Client::new();
    let res = client.post(format!("{rp_url}/register")).json(&signed_info).send().await?;
    assert!(res.status().is_success());

    // 4. Verify propagation to R
    let mut propagated = false;
    let registry_client = RegistryClient::new(true, Some(r_url.clone()));
    for _ in 0..10 {
        time::sleep(Duration::from_millis(100)).await;
        if let Ok(info) = registry_client.lookup(&did, true).await {
            assert_eq!(info.info.service_id, did);
            propagated = true;
            break;
        }
    }
    assert!(propagated, "Registration failed to propagate to parent registry");

    registry_rp.shutdown().await?;
    registry_r.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_inbound_relay() -> Result<()> {
    let logs = init_trace_collector();
    let temp_dir = tempfile::tempdir()?;
    let info_client = Client::new();
    let topo = InboundRelayTopology::spawn(temp_dir.path(), &info_client).await?;

    let mut sdk_client = topo.new_client();
    sdk_client.connect().await?;

    let response = sdk_client.request("orchestrator", "readyz", serde_json::json!({})).await?;
    assert_eq!(response.result, serde_json::json!({"status": "ok"}));

    forwarded_without_identity(&logs, &topo.target_z.did);

    sdk_client.shutdown().await?;
    topo.shutdown().await
}

#[tokio::test]
async fn unknown_host_key_sends_nothing() -> Result<()> {
    let logs = init_trace_collector();
    let temp_dir = tempfile::tempdir()?;
    let info_client = Client::new();
    let (mut registry_r, r_url) = common::spawn_registry(temp_dir.path()).await?;
    let (mut c, c_info, _c_relay_url) =
        spawn_global_coordinator(temp_dir.path(), &r_url, &info_client).await?;

    let mut sdk_client = SyneroymClient::new_with_mechanisms(
        "not-a-did".to_string(),
        vec![EndpointMechanism::Iroh {
            endpoint_addr_bytes: c_info.endpoint_addr_bytes,
            relay_url: c_info.relay_url,
        }],
    );

    sdk_client.connect().await?;
    let err =
        sdk_client.request("orchestrator", "readyz", serde_json::json!({})).await.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("no host key for 'not-a-did'"),
        "error must mention no host key for 'not-a-did': {err_str}"
    );

    let leaked = logs
        .read()
        .unwrap()
        .iter()
        .any(|l| l.contains("Forwarding preamble") && l.contains("not-a-did"));
    assert!(!leaked, "forwarder must not receive or forward any preamble for unknown host");

    sdk_client.shutdown().await?;
    c.shutdown().await?;
    registry_r.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn multi_hop_relay_with_host_key_keeps_identity_inside_encrypted_channel() -> Result<()> {
    let logs = init_trace_collector();
    let temp_dir = tempfile::tempdir()?;
    let info_client = Client::new();
    let topo = InboundRelayTopology::spawn(temp_dir.path(), &info_client).await?;

    let caller_identity = Identity::generate()?;
    let caller_did = derive_did_key(&caller_identity.public_key());
    let caller_hex = hex::encode(caller_identity.public_key().as_bytes());

    let mut sdk_client = topo.new_client().with_identity(caller_identity);
    sdk_client.connect().await?;

    let response = sdk_client.request("orchestrator", "readyz", serde_json::json!({})).await?;
    assert_eq!(response.result, serde_json::json!({"status": "ok"}));

    let lines = forwarded_without_identity(&logs, &topo.target_z.did);
    for line in &lines {
        assert!(!line.contains(&caller_did), "caller DID leaked into preamble: {line}");
        assert!(!line.contains(&caller_hex), "caller key hex leaked into preamble: {line}");
    }

    sdk_client.shutdown().await?;
    topo.shutdown().await
}

#[tokio::test]
async fn multi_hop_relay_with_wrong_host_key_fails_host_mismatch() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let info_client = Client::new();
    let topo = InboundRelayTopology::spawn(temp_dir.path(), &info_client).await?;

    let wrong_host = derive_did_key(&Identity::generate()?.public_key());
    let mut sdk_client = SyneroymClient::new_with_mechanisms(
        topo.target_z.did.clone(),
        vec![EndpointMechanism::Iroh {
            endpoint_addr_bytes: topo.c_info.endpoint_addr_bytes.clone(),
            relay_url: topo.c_info.relay_url.clone(),
        }],
    )
    .with_host(&wrong_host);

    sdk_client.connect().await?;
    let err =
        sdk_client.request("orchestrator", "readyz", serde_json::json!({})).await.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("did not prove it is host") || err_str.contains("bad handshake signature"),
        "expected bad host signature error, got: {err_str}"
    );

    sdk_client.shutdown().await?;
    topo.shutdown().await
}

#[tokio::test]
async fn test_outbound_relay() -> Result<()> {
    let logs = init_trace_collector();
    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();
    let info_client = Client::new();

    // 1. Spawn global registry R
    let (mut registry_r, r_url) = common::spawn_registry(base_path).await?;

    // 2. Spawn Global Coordinator C
    let (mut c, _c_info, c_relay_url) =
        spawn_global_coordinator(base_path, &r_url, &info_client).await?;

    // 3. Spawn Private Coordinator Cp pointing to C
    let (mut cp, cp_info) =
        spawn_private_coordinator(base_path, &r_url, &c_relay_url, &info_client).await?;

    // 4. Spawn target substrate Sx in public network under C
    let target_x = spawn_x_target_substrate(base_path, &r_url, &c_relay_url, &info_client).await?;

    // 5. Client in private network dials Cp, the forwarding entry point. Cp looks
    //    Sx up in R and dials it via Sx's home relay (C's relay).
    let mut sdk_client = SyneroymClient::new_with_mechanisms(
        target_x.did.clone(),
        vec![EndpointMechanism::Iroh {
            endpoint_addr_bytes: cp_info.endpoint_addr_bytes,
            relay_url: cp_info.relay_url,
        }],
    );

    sdk_client.connect().await?;

    // Request readyz!
    let response = sdk_client.request("orchestrator", "readyz", serde_json::json!({})).await?;
    assert_eq!(response.result, serde_json::json!({"status": "ok"}));

    forwarded_without_identity(&logs, &target_x.did);

    sdk_client.shutdown().await?;
    let _ = target_x.router.shutdown().await;
    cp.shutdown().await?;
    c.shutdown().await?;
    registry_r.shutdown().await?;
    Ok(())
}

/// A real cross-node Universal Proxy call between two full substrate nodes.
/// Unlike `test_inbound_relay`/`test_outbound_relay`, no coordinator/relay
/// infrastructure is needed here -- Sx and Sz both bind direct-address-only
/// Iroh endpoints (same machine, same process) and a lightweight HTTP
/// community registry is enough for Sx's `ProxyRouter` to resolve Sz's
/// address. Sx hosts the `proxy-test` component (imports
/// `syneroym:proxy/proxy`); Sz hosts `greeter`. Driving `call-peer` on Sx
/// exercises the full remote hop end to end -- the outbound-endpoint fix,
/// the `IrohHop`/retry loop, and proof/identity forwarding -- not just the
/// in-process `ProxyRouter::invoke` unit tests (`crates/router/src/proxy.rs`)
/// or the same-node guest-to-guest test (`proxy_dispatch.rs`).
/// What `test_cross_node_proxy_call` needs of Sz (the greeter callee) after
/// it is spawned, deployed, and registered in registry R.
struct GreeterNode {
    did: String,
    router: Router,
}

/// Spawns Sz, deploys the greeter WASM component under its own DID, and
/// registers both the component and Sz itself (via its full Iroh address) in
/// registry R, exactly as `test_cross_node_proxy_call` did inline.
///
/// Deployed *under did_z itself*, not a separate service id: the proxy
/// target (`service = did_z`) is what the community registry resolves to
/// Sz's Iroh address, so it must also be the id Sz's own local
/// `EndpointRegistry`/`AppSandboxEngine` resolve the WASM component under --
/// otherwise Sz would successfully accept the connection but find no local
/// route for it once it arrives.
async fn spawn_greeter_node(
    base_path: &Path,
    r_url: &str,
    greeter_bytes: Vec<u8>,
) -> Result<GreeterNode> {
    let identity_z = Identity::generate()?;
    let secret_z_bytes = identity_z.to_bytes();
    let did_z = derive_did_key(&identity_z.public_key());

    let mut config_z = SubstrateConfig {
        app_local_data_dir: base_path.join("data_z"),
        app_data_dir: base_path.join("user_data_z"),
        ..Default::default()
    };
    config_z.substrate.enable_bep0044_dht = false;
    config_z.resolve_paths();
    let data_store_z = registry_store::init_store(&config_z).await?;
    let endpoint_registry_z = EndpointRegistry::new(data_store_z).await?;

    // No `.online()` wait here (unlike `test_inbound_relay`/`test_outbound_relay`):
    // those configure a relay and `.online()` waits for *both* a relay
    // connection *and* a local address; this endpoint has no relay at all
    // (direct addresses only, same machine), so `.online()` would wait
    // indefinitely for the relay half of that condition. `wait_for_local_addr`
    // polls `.addr()` until it has at least one direct address instead.
    let ep_z = net_iroh::empty_endpoint_builder()
        .secret_key(SecretKey::from_bytes(&secret_z_bytes))
        .bind()
        .await?;
    let ep_z_addr = wait_for_local_addr(&ep_z).await;

    let deps_z = build_test_route_handler_deps(&config_z, &did_z, &endpoint_registry_z).await?;
    let app_sandbox_engine_z = deps_z.app_sandbox_engine.clone();
    let route_handler_z = RouteHandler::init(
        did_z.clone(),
        &config_z,
        endpoint_registry_z.clone(),
        secret_z_bytes,
        Some(ep_z.clone()),
        deps_z,
    )
    .await?;
    let router_z = Router::builder(ep_z).accept(SYNEROYM_ALPN, route_handler_z).spawn();

    app_sandbox_engine_z.deploy_wasm(&did_z, &wasm_deploy_manifest(greeter_bytes)).await.unwrap();
    endpoint_registry_z
        .register(
            did_z.clone(),
            test_constants::GREETER_INTERFACE_NAME.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: did_z.clone() },
        )
        .await?;

    // did_z itself must also resolve (the proxy target is did_z, and
    // `greeter-svc` is deployed *on* the did_z node, addressed through the
    // registry's did_z -> Iroh-mechanism mapping).
    let signed_info_z = create_signed_info_with_full_addr(&identity_z, &did_z, &ep_z_addr);
    let info_client = Client::new();
    let res = info_client.post(format!("{r_url}/register")).json(&signed_info_z).send().await?;
    assert!(res.status().is_success());

    Ok(GreeterNode { did: did_z, router: router_z })
}

/// What `test_cross_node_proxy_call` needs of Sx (the proxy-test caller)
/// after it is spawned and the `proxy-caller` component is deployed. `ep` is
/// kept only so the test can explicitly close it at the same point the
/// original inline code did.
struct ProxyCallerNode {
    ep: Endpoint,
    route_handler: RouteHandler,
}

/// Spawns Sx and deploys the `proxy-caller` WASM component, exactly as
/// `test_cross_node_proxy_call` did inline. `registry_url` points at R so
/// Sx's `ProxyRouter`'s outbound remote hop can resolve did_z.
async fn spawn_proxy_caller_node(
    base_path: &Path,
    r_url: &str,
    proxy_test_bytes: Vec<u8>,
) -> Result<ProxyCallerNode> {
    let identity_x = Identity::generate()?;
    let secret_x_bytes = identity_x.to_bytes();
    let did_x = derive_did_key(&identity_x.public_key());

    let mut config_x = SubstrateConfig {
        app_local_data_dir: base_path.join("data_x"),
        app_data_dir: base_path.join("user_data_x"),
        ..Default::default()
    };
    config_x.substrate.enable_bep0044_dht = false;
    config_x.substrate.registry_url = Some(r_url.to_string());
    config_x.resolve_paths();
    let data_store_x = registry_store::init_store(&config_x).await?;
    let endpoint_registry_x = EndpointRegistry::new(data_store_x).await?;

    let ep_x = net_iroh::empty_endpoint_builder()
        .secret_key(SecretKey::from_bytes(&secret_x_bytes))
        .bind()
        .await?;

    let deps_x = build_test_route_handler_deps(&config_x, &did_x, &endpoint_registry_x).await?;
    let app_sandbox_engine_x = deps_x.app_sandbox_engine.clone();
    let route_handler_x = RouteHandler::init(
        did_x.clone(),
        &config_x,
        endpoint_registry_x.clone(),
        secret_x_bytes,
        Some(ep_x.clone()),
        deps_x,
    )
    .await?;

    app_sandbox_engine_x
        .deploy_wasm("proxy-caller", &wasm_deploy_manifest(proxy_test_bytes))
        .await
        .unwrap();
    endpoint_registry_x
        .register(
            "proxy-caller".to_string(),
            test_constants::PROXY_TEST_DRIVER_INTERFACE.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "proxy-caller".to_string() },
        )
        .await?;

    Ok(ProxyCallerNode { ep: ep_x, route_handler: route_handler_x })
}

/// Builds the `call-peer` JSON-RPC pipeline/preamble/body that drives Sx's
/// `proxy-caller` component in `test_cross_node_proxy_call`, targeting
/// `target_service_did`'s greeter -- this is the cross-node hop: Sx's local
/// registry has no entry for the target, so `ProxyRouter::invoke` falls to
/// `invoke_remote`, resolves it via the HTTP registry, and connects directly
/// to its address.
fn build_cross_node_call_peer_request(
    target_service_did: &str,
) -> Result<(RoutePipeline, RoutePreamble, Vec<u8>)> {
    let pipeline = RoutePipeline {
        encryption: EncryptionStage::None,
        transport: TransportStage::Binary,
        adaptation: AdaptationStage::JsonRpcToWasm,
        service: ServiceStage::WasmComponent { service_id: "proxy-caller".to_string() },
    };
    let preamble = RoutePreamble::new(
        RouteTransport::Binary,
        RouteProtocol::JsonRpc,
        "proxy-caller",
        test_constants::PROXY_TEST_DRIVER_INTERFACE,
    );
    let body = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "call-peer",
        "params": {
            "service": target_service_did,
            "interface": test_constants::GREETER_INTERFACE_NAME,
            "method": "greet",
            "params": "[\"Cross-Node\"]",
            "target-kind": "service",
        },
        "id": 1,
    }))?;
    Ok((pipeline, preamble, body))
}

#[tokio::test]
async fn test_cross_node_proxy_call() -> Result<()> {
    let greeter_bytes =
        std::fs::read(test_constants::greeter_wasm_path()).expect("wasm artifact not built");
    let proxy_test_bytes =
        std::fs::read(test_constants::proxy_test_wasm_path()).expect("wasm artifact not built");

    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();

    // 1. A lightweight HTTP community registry (no DHT) -- just enough for Sx's
    //    ProxyRouter to resolve Sz's Iroh address.
    let (mut registry_r, r_url) = common::spawn_registry(base_path).await?;

    // 2. Node Sz: the greeter callee. Direct-address-only Iroh endpoint (no relay
    //    -- both nodes are local, direct addresses suffice).
    let greeter = spawn_greeter_node(base_path, &r_url, greeter_bytes).await?;

    // 3. Node Sx: the proxy-test caller.
    let proxy_caller = spawn_proxy_caller_node(base_path, &r_url, proxy_test_bytes).await?;

    // 4. Drive `call-peer` on Sx, targeting Sz's greeter.
    let (pipeline, preamble, body) = build_cross_node_call_peer_request(&greeter.did)?;

    let response_bytes = proxy_caller
        .route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, None, &body)
        .await?;
    let response: serde_json::Value = serde_json::from_slice(&response_bytes)?;
    assert!(response.get("error").is_none(), "cross-node call-peer failed: {response:?}");
    let result = response.get("result").and_then(serde_json::Value::as_str).unwrap_or_default();
    assert!(
        result.contains("Hello, Cross-Node!"),
        "expected did_z's greeter response, got: {response:?}"
    );

    let _ = greeter.router.shutdown().await;
    proxy_caller.ep.close().await;
    registry_r.shutdown().await?;
    Ok(())
}
