use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use syneroym_core::{local_registry::SubstrateEndpoint, streaming::StreamDirection};
use syneroym_identity::{Identity, substrate};
use syneroym_router::{RoutePreamble, RouteProtocol, RouteTransport};
use syneroym_rpc::{
    AuthLevel, CapabilityToken, NOT_HOSTED_HERE_RPC_CODE, OnBehalfProof,
    STREAM_UNAUTHENTICATED_RPC_CODE, framing,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

use super::helpers::*;

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("time went backwards").as_secs()
}

async fn register_test_endpoint(route_handler: &RouteHandler, service_id: &str) {
    route_handler
        .registry()
        .register(
            service_id.to_string(),
            "default".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: service_id.to_string() },
        )
        .await
        .unwrap();
}

async fn read_refusal_response(client: &mut tokio::io::DuplexStream) -> Value {
    let frame = framing::read_frame(client).await.expect("read refusal frame failed");
    serde_json::from_slice(&frame).expect("expected JSON-RPC response")
}

#[tokio::test]
async fn valid_signed_preamble_is_accepted_and_caller_authenticated() {
    let (route_handler, _) = test_route_handler().await;
    let service = Arc::new(CapturingNativeService::default());
    let service_id = "test-hosted-service";
    route_handler.register_native_service(service_id.to_string(), service.clone());
    register_test_endpoint(&route_handler, service_id).await;

    let identity = Identity::generate().unwrap();
    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&identity, now_secs());

    let (mut client, server) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let req = json!({
        "jsonrpc": "2.0",
        "method": "ping",
        "params": null,
        "id": 1
    });
    let req_bytes = serde_json::to_vec(&req).unwrap();
    framing::write_frame(&mut client, &req_bytes).await.unwrap();

    let resp_frame = framing::read_frame(&mut client).await.expect("read frame failed");
    let resp: Value = serde_json::from_slice(&resp_frame).unwrap();
    assert_eq!(resp["result"]["status"], "ok");

    let expected_did = substrate::derive_did_key(&identity.public_key());
    let captured = service.caller.lock().unwrap().take().expect("caller captured");
    assert_eq!(captured.caller_did, expected_did);
    assert_eq!(captured.auth, AuthLevel::Delegated);
    assert!(captured.proof.is_some());

    drop(client);
    let _ = handler_task.await;
}

#[tokio::test]
async fn unknown_service_returns_not_hosted_here_refusal() {
    let (route_handler, _) = test_route_handler().await;
    let identity = Identity::generate().unwrap();
    let mut preamble = RoutePreamble::new(
        RouteTransport::Binary,
        RouteProtocol::JsonRpc,
        "nonexistent-service",
        "default",
    );
    preamble.sign(&identity, now_secs());

    let (mut client, server) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let resp = read_refusal_response(&mut client).await;
    assert_eq!(resp["error"]["code"], NOT_HOSTED_HERE_RPC_CODE);

    let _ = handler_task.await;
}

#[tokio::test]
async fn expired_preamble_timestamp_is_refused() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-hosted-service";
    register_test_endpoint(&route_handler, service_id).await;

    let identity = Identity::generate().unwrap();
    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&identity, now_secs() - 1000);

    let (mut client, server) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let resp = read_refusal_response(&mut client).await;
    assert_eq!(resp["error"]["code"], STREAM_UNAUTHENTICATED_RPC_CODE);

    let _ = handler_task.await;
}

#[tokio::test]
async fn future_preamble_timestamp_is_refused() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-hosted-service";
    register_test_endpoint(&route_handler, service_id).await;

    let identity = Identity::generate().unwrap();
    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&identity, now_secs() + 1000);

    let (mut client, server) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let resp = read_refusal_response(&mut client).await;
    assert_eq!(resp["error"]["code"], STREAM_UNAUTHENTICATED_RPC_CODE);

    let _ = handler_task.await;
}

#[tokio::test]
async fn invalid_signature_is_refused() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-hosted-service";
    register_test_endpoint(&route_handler, service_id).await;

    let identity = Identity::generate().unwrap();
    let now = now_secs();
    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&identity, now);
    preamble.sig = Some("00".repeat(64));

    let (mut client, server) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let resp = read_refusal_response(&mut client).await;
    assert_eq!(resp["error"]["code"], STREAM_UNAUTHENTICATED_RPC_CODE);

    let _ = handler_task.await;
}

#[tokio::test]
async fn missing_signature_when_pubkey_present_is_refused() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-hosted-service";
    register_test_endpoint(&route_handler, service_id).await;

    let identity = Identity::generate().unwrap();
    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.pubkey = Some(hex::encode(identity.public_key().to_bytes()));
    preamble.sig = None;

    let (mut client, server) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let resp = read_refusal_response(&mut client).await;
    assert_eq!(resp["error"]["code"], STREAM_UNAUTHENTICATED_RPC_CODE);

    let _ = handler_task.await;
}

#[tokio::test]
async fn on_behalf_and_ucan_together_is_refused() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-hosted-service";
    register_test_endpoint(&route_handler, service_id).await;

    let identity = Identity::generate().unwrap();
    let p_id = Identity::generate().unwrap();
    let now = now_secs();

    let mut p_preamble = RoutePreamble::new(
        RouteTransport::Binary,
        RouteProtocol::JsonRpc,
        "orig_svc",
        "orig_iface",
    );
    p_preamble.sign(&p_id, now);
    let p_sig = p_preamble.sig.unwrap();

    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&identity, now);
    preamble.on_behalf = Some(OnBehalfProof {
        pubkey: hex::encode(p_id.public_key().to_bytes()),
        delegation: None,
        service_id: "orig_svc".to_string(),
        interface: "orig_iface".to_string(),
        ts: now,
        sig: p_sig,
    });
    let dummy_token = CapabilityToken::issue(
        &identity,
        "did:key:dummy",
        vec![],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    preamble.ucan = Some(dummy_token);

    let (mut client, server) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let resp = read_refusal_response(&mut client).await;
    assert_eq!(resp["error"]["code"], STREAM_UNAUTHENTICATED_RPC_CODE);

    assert!(handler_task.await.unwrap().is_ok());
}

#[tokio::test]
async fn valid_on_behalf_sets_on_behalf_did_but_not_anchor_did() {
    let (route_handler, _) = test_route_handler().await;
    let service = Arc::new(CapturingNativeService::default());
    let service_id = "test-hosted-service";
    route_handler.register_native_service(service_id.to_string(), service.clone());
    register_test_endpoint(&route_handler, service_id).await;

    let immediate_id = Identity::generate().unwrap();
    let p_id = Identity::generate().unwrap();
    let now = now_secs();

    let mut p_preamble = RoutePreamble::new(
        RouteTransport::Binary,
        RouteProtocol::JsonRpc,
        "orig_svc",
        "orig_iface",
    );
    p_preamble.sign(&p_id, now);
    let p_sig = p_preamble.sig.unwrap();

    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&immediate_id, now);
    preamble.on_behalf = Some(OnBehalfProof {
        pubkey: hex::encode(p_id.public_key().to_bytes()),
        delegation: None,
        service_id: "orig_svc".to_string(),
        interface: "orig_iface".to_string(),
        ts: now,
        sig: p_sig,
    });

    let (mut client, server) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let req = json!({
        "jsonrpc": "2.0",
        "method": "ping",
        "params": null,
        "id": 1
    });
    framing::write_frame(&mut client, &serde_json::to_vec(&req).unwrap()).await.unwrap();

    let resp_frame = framing::read_frame(&mut client).await.expect("read frame failed");
    let resp: Value = serde_json::from_slice(&resp_frame).unwrap();
    assert_eq!(resp["result"]["status"], "ok");

    let immediate_did = substrate::derive_did_key(&immediate_id.public_key());
    let p_did = substrate::derive_did_key(&p_id.public_key());
    let captured = service.caller.lock().unwrap().take().expect("caller captured");
    assert_eq!(captured.caller_did, immediate_did);
    assert_eq!(captured.on_behalf_did, Some(p_did));
    // The row policy and write attribution read the anchor, so P's proof
    // must not reach it: rows are read and written as the immediate caller.
    assert_eq!(captured.session.anchor_did, None);
    assert_eq!(captured.write_attribution(service_id), immediate_did);

    drop(client);
    assert!(handler_task.await.unwrap().is_ok());
}

#[tokio::test]
async fn forged_or_old_on_behalf_is_refused_by_node_check() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-hosted-service";
    register_test_endpoint(&route_handler, service_id).await;

    let immediate_id = Identity::generate().unwrap();
    let p_id = Identity::generate().unwrap();
    let other_id = Identity::generate().unwrap();
    let now = now_secs();

    // 1. Forged signature in on_behalf
    let mut p_preamble = RoutePreamble::new(
        RouteTransport::Binary,
        RouteProtocol::JsonRpc,
        "orig_svc",
        "orig_iface",
    );
    p_preamble.sign(&other_id, now);
    let forged_proof = OnBehalfProof {
        pubkey: hex::encode(p_id.public_key().to_bytes()),
        delegation: None,
        service_id: "orig_svc".to_string(),
        interface: "orig_iface".to_string(),
        ts: now,
        sig: p_preamble.sig.unwrap(),
    };

    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.sign(&immediate_id, now);
    preamble.on_behalf = Some(forged_proof);

    let (mut client, server) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server));
    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();
    let resp = read_refusal_response(&mut client).await;
    assert_eq!(resp["error"]["code"], STREAM_UNAUTHENTICATED_RPC_CODE);
    assert!(handler_task.await.unwrap().is_ok());

    // 2. Old timestamp in on_behalf (16 minutes ago, >15m window)
    let old_ts = now - (16 * 60);
    let mut p_preamble_old = RoutePreamble::new(
        RouteTransport::Binary,
        RouteProtocol::JsonRpc,
        "orig_svc",
        "orig_iface",
    );
    p_preamble_old.sign(&p_id, old_ts);
    let old_proof = OnBehalfProof {
        pubkey: hex::encode(p_id.public_key().to_bytes()),
        delegation: None,
        service_id: "orig_svc".to_string(),
        interface: "orig_iface".to_string(),
        ts: old_ts,
        sig: p_preamble_old.sig.unwrap(),
    };

    let mut preamble_old =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble_old.sign(&immediate_id, now);
    preamble_old.on_behalf = Some(old_proof);

    let (mut client2, server2) = duplex(4096);
    let handler_task2 = tokio::spawn(route_handler.handle_stream(server2));
    client2.write_all(preamble_old.to_preamble_line().as_bytes()).await.unwrap();
    let resp2 = read_refusal_response(&mut client2).await;
    assert_eq!(resp2["error"]["code"], STREAM_UNAUTHENTICATED_RPC_CODE);
    assert!(handler_task2.await.unwrap().is_ok());
}

#[tokio::test]
async fn http_preamble_refusal_returns_status_421_and_header() {
    let (route_handler, _) = test_route_handler().await;
    let identity = Identity::generate().unwrap();
    let mut preamble = RoutePreamble::new(
        RouteTransport::Http,
        RouteProtocol::JsonRpc,
        "nonexistent-service",
        "default",
    );
    preamble.sign(&identity, now_secs());

    let (mut client, server) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let mut buf = [0u8; 1024];
    let n = client.read(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf[..n]);
    assert!(text.starts_with("HTTP/1.1 421 "), "expected 421 status: {text}");
    assert!(text.contains("x-syneroym-refusal: not_hosted_here"), "expected header: {text}");

    assert!(handler_task.await.unwrap().is_ok());
}

#[tokio::test]
async fn raw_preamble_refusal_returns_not_hosted_here_line() {
    let (route_handler, _) = test_route_handler().await;
    let identity = Identity::generate().unwrap();
    let mut preamble = RoutePreamble::new(
        RouteTransport::Raw,
        RouteProtocol::JsonRpc,
        "nonexistent-service",
        "default",
    );
    preamble.dir = Some(StreamDirection::Upload);
    preamble.sign(&identity, now_secs());

    let (mut client, server) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let mut buf = [0u8; 1024];
    let n = client.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"not_hosted_here\n");

    assert!(handler_task.await.unwrap().is_ok());
}

#[tokio::test]
async fn unknown_interface_on_hosted_service_returns_32601() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-hosted-service";
    register_test_endpoint(&route_handler, service_id).await;

    let identity = Identity::generate().unwrap();
    let mut preamble = RoutePreamble::new(
        RouteTransport::Binary,
        RouteProtocol::JsonRpc,
        service_id,
        "nonexistent-interface",
    );
    preamble.sign(&identity, now_secs());

    let (mut client, server) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let resp = read_refusal_response(&mut client).await;
    assert_eq!(resp["error"]["code"], -32601);
    assert_eq!(resp["error"]["data"]["reason"], "unknown_interface");

    assert!(handler_task.await.unwrap().is_ok());
}

#[tokio::test]
async fn ucan_without_pubkey_is_refused() {
    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-hosted-service";
    register_test_endpoint(&route_handler, service_id).await;

    let identity = Identity::generate().unwrap();
    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    let dummy_token = CapabilityToken::issue(
        &identity,
        "did:key:dummy",
        vec![],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    preamble.ucan = Some(dummy_token);

    let (mut client, server) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let resp = read_refusal_response(&mut client).await;
    assert_eq!(resp["error"]["code"], STREAM_UNAUTHENTICATED_RPC_CODE);

    assert!(handler_task.await.unwrap().is_ok());
}

#[tokio::test]
async fn refused_stream_returns_ok_reaches_eof_and_never_invokes_service() {
    let (route_handler, _) = test_route_handler().await;
    let service = Arc::new(CapturingNativeService::default());
    let service_id = "test-hosted-service";
    route_handler.register_native_service(service_id.to_string(), service.clone());
    register_test_endpoint(&route_handler, service_id).await;

    let identity = Identity::generate().unwrap();
    let mut preamble =
        RoutePreamble::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, "default");
    preamble.pubkey = Some(hex::encode(identity.public_key().to_bytes()));
    preamble.sig = None;

    let (mut client, server) = duplex(4096);
    let handler_task = tokio::spawn(route_handler.clone().handle_stream(server));

    client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();

    let resp = read_refusal_response(&mut client).await;
    assert_eq!(resp["error"]["code"], STREAM_UNAUTHENTICATED_RPC_CODE);

    let mut remaining = [0u8; 128];
    let n = client.read(&mut remaining).await.unwrap();
    assert_eq!(n, 0, "stream must reach EOF after refusal");

    let res = handler_task.await.unwrap();
    assert!(res.is_ok(), "handler must return Ok(()): {res:?}");

    assert!(service.caller.lock().unwrap().is_none(), "service must not be invoked");
}

#[tokio::test]
async fn node_side_logs_redact_proof_material() {
    use std::sync::atomic::{AtomicBool, Ordering};

    use tracing_subscriber::filter::EnvFilter;

    static LOG_CHECKED: AtomicBool = AtomicBool::new(false);
    if LOG_CHECKED.swap(true, Ordering::SeqCst) {
        return;
    }

    let log_buffer = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("syneroym_router=trace"))
        .with_writer({
            let buf = Arc::clone(&log_buffer);
            struct BufferWriter(Arc<Mutex<Vec<u8>>>);
            impl Write for BufferWriter {
                fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                    self.0.lock().unwrap().extend_from_slice(b);
                    Ok(b.len())
                }
                fn flush(&mut self) -> io::Result<()> {
                    Ok(())
                }
            }
            move || BufferWriter(Arc::clone(&buf))
        })
        .finish();

    let _guard = tracing::subscriber::set_default(subscriber);

    let (route_handler, _) = test_route_handler().await;
    let service_id = "test-hosted-service";
    register_test_endpoint(&route_handler, service_id).await;

    let caller = Identity::generate().unwrap();
    let now = now_secs();

    let secret_sig = "a1b2c3d4e5f6".repeat(8);
    {
        let mut preamble = RoutePreamble::new(
            RouteTransport::Binary,
            RouteProtocol::JsonRpc,
            service_id,
            "default",
        );
        preamble.sign(&caller, now);
        preamble.on_behalf = Some(OnBehalfProof {
            pubkey: hex::encode(caller.public_key().to_bytes()),
            delegation: None,
            service_id: "orig".to_string(),
            interface: "orig".to_string(),
            ts: now,
            sig: secret_sig.clone(),
        });
        let (mut client, server) = duplex(4096);
        let handler_task = tokio::spawn(route_handler.clone().handle_stream(server));
        client.write_all(preamble.to_preamble_line().as_bytes()).await.unwrap();
        let _ = read_refusal_response(&mut client).await;
        let _ = handler_task.await;
    }

    let logs = String::from_utf8_lossy(&log_buffer.lock().unwrap()).to_string();
    assert!(!logs.contains(&secret_sig), "on_behalf signature must be redacted");
}
