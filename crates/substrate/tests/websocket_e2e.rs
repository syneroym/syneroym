#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! End-to-end tests for the WebSocket route target.

use std::collections::HashMap;

use httparse::{EMPTY_HEADER, Response as HttparseResponse, Status};
use iroh::endpoint::{RecvStream, SendStream};
use rustls::crypto::ring;
use syneroym_core::{config::AppSandboxRole, test_constants};
use syneroym_identity::Identity;
use syneroym_router::{RoutePreamble, RouteProtocol, RouteTransport};
use syneroym_sdk::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, SyneroymClient,
    TransportConnection, WasmManifest,
};

mod common;
use common::SubstrateTestContext;

fn guest_wasm_manifest(wasm_bytes: Vec<u8>, http_routes: serde_json::Value) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: Some(http_routes.to_string()),
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: None,
        },
        service_type: ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(wasm_bytes),
            hash: None,
            interfaces: vec!["syneroym:http/websocket-handler@0.1.0".to_string()],
        }),
        registry_certificate: None,
        instance_certificate: None,
    }
}

async fn deploy(client: &SyneroymClient, service_id: &str, manifest: DeployManifest) {
    let params = serde_json::to_value((service_id.to_string(), manifest)).unwrap();
    let res =
        client.request("orchestrator", "deploy", params).await.expect("deploy request failed");
    assert_eq!(res.result, serde_json::json!({"status": "deployed"}));
}

#[tokio::test]
async fn test_websocket_concurrency_limit_returns_503_with_retry_after() {
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup_with(7953, 7954, 7955, |config| {
        config.roles.app_sandbox =
            Some(AppSandboxRole { max_concurrent_websockets_per_service: 1, ..Default::default() });
    })
    .await;

    let wasm_bytes = std::fs::read(test_constants::websocket_guest_test_wasm_path())
        .expect("websocket_guest_test.wasm not built");

    let routes = serde_json::json!({
        "http_routes": [
            { "method": "GET", "path": "/ws", "public": true, "target": "websocket", "operation": "handle-upgrade" }
        ]
    });

    deploy(&ctx.substrate_client, "test-ws-limit", guest_wasm_manifest(wasm_bytes, routes)).await;

    // First connection acquires the sole permit
    let (mut send1, mut recv1) = open_http_stream(
        ctx.substrate_client.connection().as_ref().unwrap(),
        "test-ws-limit",
        None,
    )
    .await;
    let upgrade_req1 = build_websocket_upgrade_request("/ws");
    send1.write_all(&upgrade_req1).await.unwrap();

    let mut buf1 = vec![0u8; 4096];
    let n1 = recv1.read(&mut buf1).await.unwrap().unwrap();
    let resp1 = parse_http_response(&buf1[..n1]);
    assert_eq!(resp1.status, 101);

    // Second connection cannot acquire a permit and returns 503 + Retry-After
    let (mut send2, mut recv2) = open_http_stream(
        ctx.substrate_client.connection().as_ref().unwrap(),
        "test-ws-limit",
        None,
    )
    .await;
    let upgrade_req2 = build_websocket_upgrade_request("/ws");
    send2.write_all(&upgrade_req2).await.unwrap();

    let mut buf2 = vec![0u8; 4096];
    let n2 = recv2.read(&mut buf2).await.unwrap().unwrap();
    let resp2 = parse_http_response(&buf2[..n2]);
    assert_eq!(
        resp2.status, 503,
        "a websocket upgrade that cannot get an admission permit within timeout must return 503"
    );
    assert_eq!(resp2.headers.get("retry-after").map(String::as_str), Some("1"));

    drop(send1);
    drop(send2);
}

struct HttpResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn parse_http_response(raw: &[u8]) -> HttpResponse {
    let mut headers_buf = [EMPTY_HEADER; 64];
    let mut response = HttparseResponse::new(&mut headers_buf);
    let parsed = response.parse(raw).expect("failed to parse HTTP response headers");
    let Status::Complete(offset) = parsed else {
        panic!("incomplete HTTP response headers");
    };
    let status = response.code.expect("response missing a status code");
    let mut headers = HashMap::new();
    for h in response.headers.iter() {
        headers.insert(h.name.to_ascii_lowercase(), String::from_utf8_lossy(h.value).to_string());
    }
    let body = raw[offset..].to_vec();
    HttpResponse { status, headers, body }
}

async fn read_exact_buffered(recv: &mut RecvStream, unconsumed: &mut Vec<u8>, out: &mut [u8]) {
    let mut out_idx = 0;
    while out_idx < out.len() {
        if !unconsumed.is_empty() {
            let take = (out.len() - out_idx).min(unconsumed.len());
            out[out_idx..out_idx + take].copy_from_slice(&unconsumed[..take]);
            unconsumed.drain(..take);
            out_idx += take;
        } else {
            let mut tmp = [0u8; 1024];
            let n = recv.read(&mut tmp).await.unwrap().expect("unexpected EOF");
            unconsumed.extend_from_slice(&tmp[..n]);
        }
    }
}

async fn open_http_stream(
    conn: &TransportConnection,
    service_id: &str,
    pubkey: Option<&Identity>,
) -> (SendStream, RecvStream) {
    let TransportConnection::Iroh { conn, .. } = conn;
    let (mut send, recv) = conn.open_bi().await.expect("open_bi failed");
    let preamble = RoutePreamble {
        transport: RouteTransport::Http,
        protocol: RouteProtocol::JsonRpc,
        interface: "http-native".to_string(),
        service_id: service_id.to_string(),
        enc: None,
        pubkey: pubkey.map(|id| hex::encode(id.public_key().to_bytes())),
        delegation: None,
        ucan: None,
        dir: None,
    };
    send.write_all(preamble.to_preamble_line().as_bytes()).await.expect("write preamble failed");
    (send, recv)
}

fn build_websocket_upgrade_request(path: &str) -> Vec<u8> {
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: \
         Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: \
         13\r\n\r\n",
        path
    );
    req.into_bytes()
}

/// Builds a masked WebSocket text frame containing `payload`.
fn build_masked_text_frame(payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.push(0x81); // FIN + TEXT
    assert!(payload.len() < 126, "payload too long for simple frame test");
    frame.push(0x80 | (payload.len() as u8)); // Mask bit + length
    let mask = [0x12, 0x34, 0x56, 0x78];
    frame.extend_from_slice(&mask);
    for (i, &b) in payload.iter().enumerate() {
        frame.push(b ^ mask[i % 4]);
    }
    frame
}

const IROH_PORT: u16 = 7934;
const REGISTRY_PORT: u16 = 7931;
const GATEWAY_PORT: u16 = 7930;

#[tokio::test]
async fn test_websocket_echo_unicast() {
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(IROH_PORT, REGISTRY_PORT, GATEWAY_PORT).await;

    let wasm_bytes = std::fs::read(test_constants::websocket_guest_test_wasm_path())
        .expect("websocket_guest_test.wasm not built");

    let routes = serde_json::json!({
        "http_routes": [
            { "method": "GET", "path": "/ws", "public": true, "target": "websocket", "operation": "handle-upgrade" }
        ]
    });

    deploy(&ctx.substrate_client, "test-ws-service", guest_wasm_manifest(wasm_bytes, routes)).await;

    let (mut send, mut recv) = open_http_stream(
        ctx.substrate_client.connection().as_ref().unwrap(),
        "test-ws-service",
        None,
    )
    .await;

    // Send upgrade request
    let upgrade_req = build_websocket_upgrade_request("/ws");
    send.write_all(&upgrade_req).await.unwrap();

    // Read 101 Switching Protocols response
    let mut buf = vec![0u8; 4096];
    let n = recv.read(&mut buf).await.unwrap().unwrap();
    let resp = parse_http_response(&buf[..n]);
    assert_eq!(resp.status, 101);
    assert_eq!(resp.headers.get("upgrade").map(|s| s.as_str()), Some("websocket"));

    let mut unconsumed = resp.body;

    // Test on-open welcome message (unmasked text frame)
    // 0x81 = FIN + TEXT, 0x07 = length 7 ("welcome")
    let mut frame_header = [0u8; 2];
    read_exact_buffered(&mut recv, &mut unconsumed, &mut frame_header).await;
    assert_eq!(frame_header[0], 0x81);
    assert_eq!(frame_header[1], 0x07);

    let mut payload = vec![0u8; 7];
    read_exact_buffered(&mut recv, &mut unconsumed, &mut payload).await;
    assert_eq!(&payload, b"welcome");

    // Send a masked "hello" text frame
    let masked_hello = build_masked_text_frame(b"hello");
    send.write_all(&masked_hello).await.unwrap();

    // Read unmasked "hello" text response
    read_exact_buffered(&mut recv, &mut unconsumed, &mut frame_header).await;
    assert_eq!(frame_header[0], 0x81);
    assert_eq!(frame_header[1], 0x05); // length 5

    let mut payload = vec![0u8; 5];
    read_exact_buffered(&mut recv, &mut unconsumed, &mut payload).await;
    assert_eq!(&payload, b"hello");

    // Send a masked binary frame
    let mut masked_bin = build_masked_text_frame(&[10, 20, 30, 40]);
    masked_bin[0] = 0x82; // BINARY opcode
    send.write_all(&masked_bin).await.unwrap();

    // Read unmasked binary response
    read_exact_buffered(&mut recv, &mut unconsumed, &mut frame_header).await;
    assert_eq!(frame_header[0], 0x82);
    assert_eq!(frame_header[1], 0x04);

    let mut bin_payload = vec![0u8; 4];
    read_exact_buffered(&mut recv, &mut unconsumed, &mut bin_payload).await;
    assert_eq!(&bin_payload, &[10, 20, 30, 40]);

    // Close cleanly
    drop(send);
}

#[tokio::test]
async fn test_websocket_broadcast_pubsub() {
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(7924, 7921, 7920).await;

    let wasm_bytes = std::fs::read(test_constants::websocket_guest_test_wasm_path())
        .expect("websocket_guest_test.wasm not built");

    let routes = serde_json::json!({
        "http_routes": [
            { "method": "GET", "path": "/ws", "public": true, "target": "websocket", "operation": "handle-upgrade", "topic": "test/topic" }
        ]
    });

    deploy(&ctx.substrate_client, "test-ws-pubsub", guest_wasm_manifest(wasm_bytes, routes)).await;

    let (mut send, mut recv) = open_http_stream(
        ctx.substrate_client.connection().as_ref().unwrap(),
        "test-ws-pubsub",
        None,
    )
    .await;

    let upgrade_req = build_websocket_upgrade_request("/ws");
    send.write_all(&upgrade_req).await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = recv.read(&mut buf).await.unwrap().unwrap();
    let resp = parse_http_response(&buf[..n]);
    assert_eq!(resp.status, 101);

    let mut unconsumed = resp.body;

    // Read on-open welcome message (Text frame)
    let mut frame_header = [0u8; 2];
    read_exact_buffered(&mut recv, &mut unconsumed, &mut frame_header).await;
    assert_eq!(frame_header[0], 0x81); // TEXT frame
    let mut payload = vec![0u8; 7];
    read_exact_buffered(&mut recv, &mut unconsumed, &mut payload).await;

    // Publish a message to the topic via SyneroymClient
    let mut publisher = SyneroymClient::new_with_mechanisms(
        "test-ws-pubsub".to_string(),
        ctx.substrate_mechanisms.clone(),
    )
    .with_registry_dht(false);
    publisher.connect().await.unwrap();

    publisher
        .request(
            "messaging",
            "publish",
            serde_json::json!({
                "topic": "test/topic",
                "payload": b"pubsub_hello".to_vec()
            }),
        )
        .await
        .unwrap();

    // Read the pushed frame from WebSocket (valid UTF-8 sent as Text frame)
    read_exact_buffered(&mut recv, &mut unconsumed, &mut frame_header).await;
    assert_eq!(frame_header[0], 0x81); // TEXT frame
    assert_eq!(frame_header[1], 12); // length of "pubsub_hello"

    let mut payload = vec![0u8; 12];
    read_exact_buffered(&mut recv, &mut unconsumed, &mut payload).await;
    assert_eq!(&payload, b"pubsub_hello");
}

#[tokio::test]
async fn test_websocket_upgrade_rejects_unauthenticated_anonymous_on_private_route() {
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(7950, 7951, 7952).await;

    let wasm_bytes = std::fs::read(test_constants::websocket_guest_test_wasm_path())
        .expect("websocket_guest_test.wasm not built");

    // Private route (public: false)
    let routes = serde_json::json!({
        "http_routes": [
            { "method": "GET", "path": "/private-ws", "public": false, "target": "websocket", "operation": "handle-upgrade" }
        ]
    });

    deploy(&ctx.substrate_client, "test-ws-auth", guest_wasm_manifest(wasm_bytes, routes)).await;

    // Anonymous caller (no delegation/identity)
    let (mut send, mut recv) =
        open_http_stream(ctx.substrate_client.connection().as_ref().unwrap(), "test-ws-auth", None)
            .await;

    let upgrade_req = build_websocket_upgrade_request("/private-ws");
    send.write_all(&upgrade_req).await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = recv.read(&mut buf).await.unwrap().unwrap();
    let resp = parse_http_response(&buf[..n]);
    assert_eq!(
        resp.status, 401,
        "anonymous request to private websocket route must return 401 Unauthorized"
    );
}

#[tokio::test]
async fn test_websocket_teardown_on_undeploy() {
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(7956, 7957, 7958).await;

    let wasm_bytes = std::fs::read(test_constants::websocket_guest_test_wasm_path())
        .expect("websocket_guest_test.wasm not built");

    let routes = serde_json::json!({
        "http_routes": [
            { "method": "GET", "path": "/ws", "public": true, "target": "websocket", "operation": "handle-upgrade" }
        ]
    });

    deploy(&ctx.substrate_client, "test-ws-undeploy", guest_wasm_manifest(wasm_bytes, routes))
        .await;

    let (mut send, mut recv) = open_http_stream(
        ctx.substrate_client.connection().as_ref().unwrap(),
        "test-ws-undeploy",
        None,
    )
    .await;

    let upgrade_req = build_websocket_upgrade_request("/ws");
    send.write_all(&upgrade_req).await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = recv.read(&mut buf).await.unwrap().unwrap();
    let resp = parse_http_response(&buf[..n]);
    assert_eq!(resp.status, 101);

    // Undeploy the service while the websocket connection is active
    let params = serde_json::json!(["test-ws-undeploy", 1]);
    let res = ctx
        .substrate_client
        .request("orchestrator", "undeploy", params)
        .await
        .expect("undeploy request failed");
    assert_eq!(res.result, serde_json::json!({"status": "undeployed"}));

    // Drop send to close connection
    drop(send);
}
