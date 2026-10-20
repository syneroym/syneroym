//! Integration tests verifying signed preamble and delegation chain forwarding
//! across real Iroh nodes.
#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    path::Path,
    sync::{Arc, Mutex, OnceLock, RwLock, Weak},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use async_trait::async_trait;
use iroh::{Endpoint, SecretKey, protocol::Router};
use reqwest::Client;
use serde_json::json;
use syneroym_core::{
    config::{RetryPolicy, SubstrateConfig},
    local_registry::SubstrateEndpoint,
};
use syneroym_data_db::registry_store;
use syneroym_identity::{DelegationCertificate, Identity, substrate::derive_did_key};
use syneroym_router::{
    IrohHop, RemoteHop, RouteHandler, RoutePreamble, RouteProtocol, RouteTransport, SYNEROYM_ALPN,
};
use syneroym_rpc::{
    CallOrigin, CallerContext, JsonRpcRequest, NOT_HOSTED_HERE_RPC_CODE, NativeInvocation,
    NativeResponse, NativeService, OnBehalfProof, ProxyError, ProxyProtocol, ProxyRequest,
    RpcResult, STREAM_UNAUTHENTICATED_RPC_CODE, ServiceProxy, framing,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod common;
use common::{
    nodes::{
        build_delegated_proxy_request, build_test_route_handler_deps,
        create_signed_info_with_full_addr, spawn_global_coordinator,
        spawn_native_capability_callee_node, spawn_native_capability_caller_node,
        wait_for_local_addr,
    },
    spawn_registry,
};

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("time went backwards").as_secs()
}

#[derive(Default)]
struct SharedLogBuffer {
    lines: Arc<RwLock<Vec<String>>>,
}

struct LogWriter {
    buffer: Arc<RwLock<Vec<String>>>,
}

impl std::io::Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(s) = std::str::from_utf8(buf) {
            let mut lines = self.buffer.write().unwrap();
            lines.push(s.to_string());
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

static SHARED_LOGS: OnceLock<SharedLogBuffer> = OnceLock::new();

fn init_trace_collector() -> Arc<RwLock<Vec<String>>> {
    let shared = SHARED_LOGS.get_or_init(|| {
        let lines = Arc::new(RwLock::new(Vec::new()));
        let lines_clone = lines.clone();
        let make_writer = move || LogWriter { buffer: lines_clone.clone() };
        let layer = tracing_subscriber::fmt::layer().with_writer(make_writer).with_ansi(false);
        let filter = tracing_subscriber::filter::EnvFilter::new("trace");
        let _ = tracing_subscriber::registry().with(layer).with(filter).try_init();
        SharedLogBuffer { lines }
    });
    shared.lines.clone()
}

#[derive(Debug)]
struct IntermediateForwardingService {
    captured_caller: Mutex<Option<CallerContext>>,
    target_did: String,
    service_proxy: OnceLock<Weak<dyn ServiceProxy>>,
}

#[async_trait]
impl NativeService for IntermediateForwardingService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        *self.captured_caller.lock().unwrap() = Some(invocation.caller.clone());
        let proxy = self
            .service_proxy
            .get()
            .expect("service proxy must be initialized")
            .upgrade()
            .expect("service proxy must be alive");

        let req = ProxyRequest {
            target_service: self.target_did.clone(),
            interface: "data-layer".to_string(),
            method: "query".to_string(),
            params: json!({}),
            caller: invocation.caller,
            origin: CallOrigin::Native { service_id: None },
            protocol: ProxyProtocol::JsonRpcV1,
            idempotency_key: None,
            idempotent: false,
            timeout: Some(Duration::from_secs(5)),
        };

        proxy.invoke(req).await.map_err(|e| match e {
            ProxyError::Callee { code, message, .. } => {
                syneroym_rpc::RpcError::Custom(code, message, None)
            }
            other => syneroym_rpc::RpcError::Custom(-32603, other.to_string(), None),
        })?;

        Ok(NativeResponse { payload: json!({"status": "forwarded"}) })
    }
}

struct IntermediateNode {
    did: String,
    router: Router,
    service: Arc<IntermediateForwardingService>,
}

async fn spawn_intermediate_node(
    base_path: &Path,
    r_url: &str,
    target_did: &str,
) -> Result<IntermediateNode> {
    let identity_b = Identity::generate()?;
    let secret_b_bytes = identity_b.to_bytes();
    let did_b = derive_did_key(&identity_b.public_key());

    let mut config_b = SubstrateConfig {
        app_local_data_dir: base_path.join("data_b"),
        app_data_dir: base_path.join("user_data_b"),
        ..Default::default()
    };
    config_b.substrate.enable_bep0044_dht = false;
    config_b.substrate.registry_url = Some(r_url.to_string());
    config_b.resolve_paths();
    let data_store_b = registry_store::init_store(&config_b).await?;
    let endpoint_registry_b =
        syneroym_core::local_registry::EndpointRegistry::new(data_store_b).await?;

    let ep_b =
        Endpoint::empty_builder().secret_key(SecretKey::from_bytes(&secret_b_bytes)).bind().await?;
    let ep_b_addr = wait_for_local_addr(&ep_b).await;

    let deps_b = build_test_route_handler_deps(&config_b, &did_b, &endpoint_registry_b).await?;
    let app_sandbox_engine_b = deps_b.app_sandbox_engine.clone();
    let native_dispatch_b = deps_b.native_dispatch.clone();

    let forwarding_service = Arc::new(IntermediateForwardingService {
        captured_caller: Mutex::new(None),
        target_did: target_did.to_string(),
        service_proxy: OnceLock::new(),
    });

    native_dispatch_b
        .insert("chain-svc-b".to_string(), forwarding_service.clone() as Arc<dyn NativeService>);
    endpoint_registry_b
        .register(
            did_b.clone(),
            "chain".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "chain-svc-b".to_string() },
        )
        .await?;

    let route_handler_b = RouteHandler::init(
        did_b.clone(),
        &config_b,
        endpoint_registry_b.clone(),
        secret_b_bytes,
        Some(ep_b.clone()),
        deps_b,
    )
    .await?;

    let weak_proxy = app_sandbox_engine_b
        .service_proxy
        .get()
        .expect("service proxy is set by RouteHandler::init");
    forwarding_service
        .service_proxy
        .set(weak_proxy.clone())
        .map_err(|_| anyhow::anyhow!("service proxy already set"))?;

    let router_b = Router::builder(ep_b).accept(SYNEROYM_ALPN, route_handler_b).spawn();

    let signed_info_b = create_signed_info_with_full_addr(&identity_b, &did_b, &ep_b_addr);
    let info_client = Client::new();
    let res = info_client.post(format!("{r_url}/register")).json(&signed_info_b).send().await?;
    assert!(res.status().is_success());

    Ok(IntermediateNode { did: did_b, router: router_b, service: forwarding_service })
}

#[tokio::test]
async fn cross_node_native_capability_identity_forwarding() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();

    let (mut registry_r, r_url) = spawn_registry(base_path).await?;
    let callee = spawn_native_capability_callee_node(base_path, &r_url).await?;
    let caller = spawn_native_capability_caller_node(base_path, &r_url).await?;

    let (master_did, req) = build_delegated_proxy_request(&r_url, &callee.did).await?;

    caller
        .service_proxy
        .invoke(req)
        .await
        .map_err(|e| anyhow::anyhow!("proxy call failed: {e}"))?;

    assert_eq!(
        callee.capturing_service.captured_caller_did.lock().unwrap().as_deref(),
        Some(caller.did.as_str()),
        "destination sees caller node DID"
    );
    assert_eq!(
        callee.capturing_service.captured_anchor_did.lock().unwrap().as_deref(),
        Some(master_did.as_str()),
        "destination sees delegated master anchor DID"
    );

    let _ = caller.route_handler;
    let _ = callee.router.shutdown().await;
    caller.ep.close().await;
    registry_r.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn chain_three_nodes_two_on_behalf_hops() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();

    let (mut registry_r, r_url) = spawn_registry(base_path).await?;
    let sc = spawn_native_capability_callee_node(base_path, &r_url).await?;
    let sb = spawn_intermediate_node(base_path, &r_url, &sc.did).await?;
    let sa = spawn_native_capability_caller_node(base_path, &r_url).await?;

    let (master_did, mut req) = build_delegated_proxy_request(&r_url, &sb.did).await?;
    req.interface = "chain".to_string();

    sa.service_proxy
        .invoke(req)
        .await
        .map_err(|e| anyhow::anyhow!("chain proxy call failed: {e}"))?;

    let sb_captured = sb.service.captured_caller.lock().unwrap().take().unwrap();
    assert_eq!(sb_captured.caller_did, sa.did);
    assert_eq!(sb_captured.session.anchor_did.as_deref(), Some(master_did.as_str()));

    let sc_caller_did = sc.capturing_service.captured_caller_did.lock().unwrap().take().unwrap();
    let sc_anchor_did = sc.capturing_service.captured_anchor_did.lock().unwrap().take().unwrap();
    assert_eq!(sc_caller_did, sb.did);
    assert_eq!(sc_anchor_did, master_did);

    let _ = sa.route_handler;
    let _ = sb.router.shutdown().await;
    let _ = sc.router.shutdown().await;
    sa.ep.close().await;
    registry_r.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn chain_with_forged_proof_is_refused() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();

    let (mut registry_r, r_url) = spawn_registry(base_path).await?;
    let sc = spawn_native_capability_callee_node(base_path, &r_url).await?;
    let sb = spawn_intermediate_node(base_path, &r_url, &sc.did).await?;
    let sa = spawn_native_capability_caller_node(base_path, &r_url).await?;

    let (_master_did, mut req) = build_delegated_proxy_request(&r_url, &sb.did).await?;
    req.interface = "chain".to_string();

    let mut proof = req.caller.proof.take().unwrap();
    if let Some(ref mut signed) = proof.signed {
        signed.sig_hex = "00".repeat(64);
    }
    req.caller.proof = Some(proof);

    let err = sa.service_proxy.invoke(req).await.unwrap_err();
    match err {
        ProxyError::Callee { code, .. } => {
            assert_eq!(code, STREAM_UNAUTHENTICATED_RPC_CODE);
        }
        other => panic!("expected callee refusal with -32097, got: {other:?}"),
    }

    assert!(sc.capturing_service.captured_caller_did.lock().unwrap().is_none());

    let _ = sa.route_handler;
    let _ = sb.router.shutdown().await;
    let _ = sc.router.shutdown().await;
    sa.ep.close().await;
    registry_r.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn chain_with_expired_proof_is_refused() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();

    let (mut registry_r, r_url) = spawn_registry(base_path).await?;
    let sc = spawn_native_capability_callee_node(base_path, &r_url).await?;
    let sb = spawn_intermediate_node(base_path, &r_url, &sc.did).await?;
    let sa = spawn_native_capability_caller_node(base_path, &r_url).await?;

    let (_master_did, mut req) = build_delegated_proxy_request(&r_url, &sb.did).await?;
    req.interface = "chain".to_string();

    let mut proof = req.caller.proof.take().unwrap();
    if let Some(ref mut signed) = proof.signed {
        signed.ts = now_secs() - 1000;
    }
    req.caller.proof = Some(proof);

    let err = sa.service_proxy.invoke(req).await.unwrap_err();
    match err {
        ProxyError::Callee { code, .. } => {
            assert_eq!(code, STREAM_UNAUTHENTICATED_RPC_CODE);
        }
        other => panic!("expected callee refusal with -32097, got: {other:?}"),
    }

    assert!(sc.capturing_service.captured_caller_did.lock().unwrap().is_none());

    let _ = sa.route_handler;
    let _ = sb.router.shutdown().await;
    let _ = sc.router.shutdown().await;
    sa.ep.close().await;
    registry_r.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn large_request_to_unhosted_service_refused_without_hang() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();

    let (mut registry_r, r_url) = spawn_registry(base_path).await?;
    let sb = spawn_native_capability_callee_node(base_path, &r_url).await?;
    let sa = spawn_native_capability_caller_node(base_path, &r_url).await?;

    let hop = IrohHop::new(Some(sa.ep.clone()), RetryPolicy::default());
    let addr = wait_for_local_addr(sb.router.endpoint()).await;

    let large_payload = "a".repeat(4 * 1024 * 1024);
    let params = json!({"payload": large_payload});
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "query".to_string(),
        params,
        id: Some(json!(1)),
        idempotency_key: None,
    };

    let preamble = RoutePreamble::new(
        RouteTransport::Binary,
        RouteProtocol::JsonRpc,
        "nonexistent-unhosted-service",
        "default",
    );

    let err =
        hop.call(&addr, &preamble, None, &request, Duration::from_secs(10)).await.unwrap_err();

    match err {
        ProxyError::Callee { code, .. } => {
            assert_eq!(code, NOT_HOSTED_HERE_RPC_CODE);
        }
        other => panic!("expected Callee refusal with -32096, got: {other:?}"),
    }

    let _ = sa.route_handler;
    let _ = sb.router.shutdown().await;
    sa.ep.close().await;
    registry_r.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn forwarding_log_redaction() -> Result<()> {
    let log_lines = init_trace_collector();

    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();

    let (mut registry_r, r_url) = spawn_registry(base_path).await?;
    let info_client = Client::new();
    let (mut coord, _coord_info, _relay_url) =
        spawn_global_coordinator(base_path, &r_url, &info_client).await?;
    let target = spawn_native_capability_callee_node(base_path, &r_url).await?;

    let coord_addr = coord.endpoint_addr().unwrap();
    let client_ep = Endpoint::empty_builder().bind().await?;

    let master = Identity::generate()?;
    let _master_did = derive_did_key(&master.public_key());
    let instance = Identity::generate()?;
    let cert =
        DelegationCertificate::issue(&master, instance.public_key(), 3600, "routing".to_string())?;
    let secret_cert_json = cert.to_json()?;

    let mut preamble = RoutePreamble::new(
        RouteTransport::Binary,
        RouteProtocol::JsonRpc,
        &target.did,
        "data-layer",
    );
    preamble.delegation = Some(cert);
    let secret_sig_now = now_secs();
    preamble.sign(&instance, secret_sig_now);

    let secret_sig_hex = preamble.sig.clone().unwrap();

    let secret_on_behalf = OnBehalfProof {
        pubkey: hex::encode(instance.public_key().to_bytes()),
        delegation: Some(secret_cert_json.clone()),
        service_id: target.did.clone(),
        interface: "data-layer".to_string(),
        ts: secret_sig_now,
        sig: secret_sig_hex.clone(),
    };
    let secret_on_behalf_hex = hex::encode(serde_json::to_vec(&secret_on_behalf).unwrap());
    preamble.on_behalf = Some(secret_on_behalf);

    let conn = client_ep.connect(coord_addr, SYNEROYM_ALPN).await?;
    let (mut send, mut recv) = conn.open_bi().await?;

    let preamble_line = preamble.to_preamble_line();
    send.write_all(preamble_line.as_bytes()).await?;

    let req = json!({
        "jsonrpc": "2.0",
        "method": "query",
        "params": {},
        "id": 1
    });
    framing::write_frame(&mut send, &serde_json::to_vec(&req).unwrap()).await?;

    let _ = framing::read_frame(&mut recv).await;

    drop(send);
    drop(recv);

    let captured = log_lines.read().unwrap().join("\n");
    assert!(!captured.contains(&secret_sig_hex), "log must not leak raw signature hex");
    assert!(!captured.contains(&secret_cert_json), "log must not leak delegation certificate JSON");
    assert!(!captured.contains(&secret_on_behalf_hex), "log must not leak on_behalf hex");

    let _ = target.router.shutdown().await;
    let _ = coord.shutdown().await;
    client_ep.close().await;
    registry_r.shutdown().await?;
    Ok(())
}
