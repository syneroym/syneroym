#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! M06A A3 end-to-end tests: WebSocket route target.

use std::collections::HashMap;

use httparse::{EMPTY_HEADER, Response as HttparseResponse, Status};
use iroh::endpoint::{RecvStream, SendStream};
use rustls::crypto::ring;
use syneroym_core::test_constants;
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
    assert_eq!(res.result, serde_json::json!({"status": "deployed"}), "deploy did not succeed");
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

    // Test on-open welcome message (unmasked binary frame)
    // 0x82 = FIN + BINARY, 0x07 = length 7 ("welcome")
    let mut frame_header = [0u8; 2];
    recv.read_exact(&mut frame_header).await.unwrap();
    assert_eq!(frame_header[0], 0x82);
    assert_eq!(frame_header[1], 0x07);

    let mut payload = vec![0u8; 7];
    recv.read_exact(&mut payload).await.unwrap();
    assert_eq!(&payload, b"welcome");

    // Send a masked "hello" text frame, but it will be echoed as binary frame!
    // Wait, if I send a TEXT frame, does the server's `on_message` receive it and
    // echo it? Let's send a masked binary frame to be clean.
    let mut masked_hello = build_masked_text_frame(b"hello");
    masked_hello[0] = 0x82; // Make it BINARY instead of TEXT
    send.write_all(&masked_hello).await.unwrap();

    // Read unmasked "hello" response
    recv.read_exact(&mut frame_header).await.unwrap();
    assert_eq!(frame_header[0], 0x82);
    assert_eq!(frame_header[1], 0x05); // length 5

    let mut payload = vec![0u8; 5];
    recv.read_exact(&mut payload).await.unwrap();
    assert_eq!(&payload, b"hello");

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

    // Read on-open welcome message
    let mut frame_header = [0u8; 2];
    recv.read_exact(&mut frame_header).await.unwrap();
    assert_eq!(frame_header[0], 0x82); // BINARY frame
    let mut payload = vec![0u8; 7];
    recv.read_exact(&mut payload).await.unwrap();

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

    // Read the pushed frame from WebSocket
    recv.read_exact(&mut frame_header).await.unwrap();
    assert_eq!(frame_header[0], 0x82); // Binary frame
    assert_eq!(frame_header[1], 12); // length of "pubsub_hello"

    let mut payload = vec![0u8; 12];
    recv.read_exact(&mut payload).await.unwrap();
    assert_eq!(&payload, b"pubsub_hello");
}
