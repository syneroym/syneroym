#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! End-to-end tests for `miniapp-demo1-wasm` fixture.
//!
//! Exercises static UI serving with zero guest instantiations, REST APIs
//! via guest HTTP handler, file streaming upload/download, SSE live updates,
//! and WebSocket echo/broadcast.

use std::{collections::HashMap, time::Duration};

use httparse::{EMPTY_HEADER, Response as HttparseResponse, Status};
use iroh::endpoint::{RecvStream, SendStream};
use rustls::crypto::ring;
use syneroym_core::test_constants;
use syneroym_identity::Identity;
use syneroym_observability::MemoryRecorder;
use syneroym_router::{RoutePreamble, RouteProtocol, RouteTransport};
use syneroym_sdk::{
    ArtifactSource, AssetBundle, DeployManifest, ServiceConfig, ServiceType, SyneroymClient,
    TransportConnection, Visibility, WasmManifest,
};
use tokio::time::timeout;

mod common;
use common::SubstrateTestContext;

fn miniapp_manifest_with_assets(
    wasm_bytes: Vec<u8>,
    http_routes: serde_json::Value,
    archive: Vec<u8>,
) -> DeployManifest {
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
            assets: Some(AssetBundle {
                archive: ArtifactSource::Binary(archive),
                hash: None,
                visibility: Some(Visibility::Public),
            }),
        },
        service_type: ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(wasm_bytes),
            hash: None,
            interfaces: vec![
                "syneroym:http/incoming-handler@0.1.0".to_string(),
                "syneroym:http/websocket-handler@0.1.0".to_string(),
                "syneroym:messaging/guest-api@0.1.0".to_string(),
                "syneroym:messaging/stream-types@0.1.0".to_string(),
            ],
        }),
        registry_certificate: None,
        instance_certificate: None,
    }
}

fn make_asset_archive(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut builder = tar::Builder::new(Vec::new());
    for (path, data) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, path, *data).unwrap();
    }
    let tar_bytes = builder.into_inner().unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&tar_bytes).unwrap();
    encoder.finish().unwrap()
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

fn decode_chunked_body(raw_body: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    let mut pos = 0;
    while pos < raw_body.len() {
        let Some(line_end) = raw_body[pos..].windows(2).position(|w| w == b"\r\n") else {
            break;
        };
        let line = &raw_body[pos..pos + line_end];
        let Ok(size_str) = std::str::from_utf8(line) else {
            break;
        };
        let Ok(size) = usize::from_str_radix(size_str.trim(), 16) else {
            break;
        };
        if size == 0 {
            break;
        }
        let chunk_start = pos + line_end + 2;
        let chunk_end = chunk_start + size;
        if chunk_end > raw_body.len() {
            break;
        }
        decoded.extend_from_slice(&raw_body[chunk_start..chunk_end]);
        pos = chunk_end + 2;
    }
    decoded
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
    let body_bytes = &raw[offset..];
    let body = if headers.get("transfer-encoding").is_some_and(|v| v.contains("chunked")) {
        decode_chunked_body(body_bytes)
    } else {
        body_bytes.to_vec()
    };
    HttpResponse { status, headers, body }
}

async fn read_full_http_response(recv: &mut RecvStream) -> HttpResponse {
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match timeout(Duration::from_secs(5), recv.read(&mut buf)).await {
            Ok(Ok(Some(n))) if n > 0 => {
                raw.extend_from_slice(&buf[..n]);
                let mut headers_buf = [EMPTY_HEADER; 64];
                let mut response = HttparseResponse::new(&mut headers_buf);
                if let Ok(Status::Complete(offset)) = response.parse(&raw) {
                    let is_chunked = response.headers.iter().any(|h| {
                        h.name.eq_ignore_ascii_case("transfer-encoding")
                            && String::from_utf8_lossy(h.value).contains("chunked")
                    });
                    let content_length: Option<usize> = response.headers.iter().find_map(|h| {
                        if h.name.eq_ignore_ascii_case("content-length") {
                            std::str::from_utf8(h.value).ok().and_then(|v| v.trim().parse().ok())
                        } else {
                            None
                        }
                    });

                    if let Some(cl) = content_length {
                        if raw.len() >= offset + cl {
                            break;
                        }
                    } else if is_chunked {
                        if raw[offset..].windows(5).any(|w| w == b"0\r\n\r\n") {
                            break;
                        }
                    } else {
                        break;
                    }
                }
            }
            _ => break,
        }
    }
    parse_http_response(&raw)
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

fn counter_value(name: &str) -> u64 {
    MemoryRecorder::global()
        .expect("global MemoryRecorder must be installed by the substrate under test")
        .snapshot()
        .counters
        .get(name)
        .copied()
        .unwrap_or(0)
}

fn build_websocket_upgrade_request(path: &str) -> Vec<u8> {
    format!(
        "GET {} HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: \
         Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: \
         13\r\n\r\n",
        path
    )
    .into_bytes()
}

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

#[tokio::test]
async fn test_miniapp_demo1_wasm_static_asset_serving_zero_instantiations() {
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(23100, 23101, 23102).await;
    ctx.substrate_client.inject_kek("21".repeat(32)).await.expect("inject_kek failed");

    let wasm_bytes = std::fs::read(test_constants::miniapp_demo1_wasm_path())
        .expect("miniapp_demo1_wasm.wasm not built");

    let index_html =
        b"<!DOCTYPE html><html><body><h1>Hello world from demo1-instance0</h1></body></html>";
    let page1_html = b"<!DOCTYPE html><html><body><h1>Static page 1</h1></body></html>";
    let archive = make_asset_archive(&[("index.html", index_html), ("page1.html", page1_html)]);

    let routes: serde_json::Value =
        serde_json::from_str(test_constants::MINIAPP_DEMO1_WASM_ROUTES_JSON).unwrap();

    deploy(
        &ctx.substrate_client,
        "demo-app",
        miniapp_manifest_with_assets(wasm_bytes, routes, archive),
    )
    .await;

    let baseline = counter_value("syneroym_wasm_instances_total");

    // 1. GET /
    let (mut send, mut recv) =
        open_http_stream(ctx.substrate_client.connection().as_ref().unwrap(), "demo-app", None)
            .await;
    send.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
    let resp = read_full_http_response(&mut recv).await;
    assert_eq!(resp.status, 200);
    assert!(resp.headers.get("content-type").unwrap().starts_with("text/html"));
    assert!(String::from_utf8_lossy(&resp.body).contains("Hello world from demo1-instance0"));
    let etag = resp.headers.get("etag").expect("expected ETag header").clone();

    // 2. GET /page1.html
    let (mut send2, mut recv2) =
        open_http_stream(ctx.substrate_client.connection().as_ref().unwrap(), "demo-app", None)
            .await;
    send2.write_all(b"GET /page1.html HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
    let resp2 = read_full_http_response(&mut recv2).await;
    assert_eq!(resp2.status, 200);
    assert!(String::from_utf8_lossy(&resp2.body).contains("Static page 1"));

    // 3. Repeat GET / with If-None-Match -> 304 Not Modified
    let (mut send3, mut recv3) =
        open_http_stream(ctx.substrate_client.connection().as_ref().unwrap(), "demo-app", None)
            .await;
    let req3 = format!("GET / HTTP/1.1\r\nHost: localhost\r\nIf-None-Match: {etag}\r\n\r\n");
    send3.write_all(req3.as_bytes()).await.unwrap();
    let resp3 = read_full_http_response(&mut recv3).await;
    assert_eq!(resp3.status, 304);

    // Zero guest instantiations for static assets
    assert_eq!(counter_value("syneroym_wasm_instances_total"), baseline);
}

#[tokio::test]
async fn test_miniapp_demo1_wasm_comments_spa_page() {
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(23103, 23104, 23105).await;
    ctx.substrate_client.inject_kek("21".repeat(32)).await.expect("inject_kek failed");

    let wasm_bytes = std::fs::read(test_constants::miniapp_demo1_wasm_path())
        .expect("miniapp_demo1_wasm.wasm not built");
    let archive = make_asset_archive(&[("index.html", b"<h1>Static fallback</h1>")]);
    let routes: serde_json::Value =
        serde_json::from_str(test_constants::MINIAPP_DEMO1_WASM_ROUTES_JSON).unwrap();

    deploy(
        &ctx.substrate_client,
        "demo-spa",
        miniapp_manifest_with_assets(wasm_bytes, routes, archive),
    )
    .await;

    let (mut send, mut recv) =
        open_http_stream(ctx.substrate_client.connection().as_ref().unwrap(), "demo-spa", None)
            .await;
    send.write_all(b"GET /comments HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
    let resp = read_full_http_response(&mut recv).await;
    assert_eq!(resp.status, 200);
    assert!(resp.headers.get("content-type").unwrap().contains("text/html"));
    assert!(String::from_utf8_lossy(&resp.body).contains("id=\"root\""));
}

#[tokio::test]
async fn test_miniapp_demo1_wasm_rest_comments_lifecycle() {
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(23106, 23107, 23108).await;
    ctx.substrate_client.inject_kek("21".repeat(32)).await.expect("inject_kek failed");

    let wasm_bytes = std::fs::read(test_constants::miniapp_demo1_wasm_path())
        .expect("miniapp_demo1_wasm.wasm not built");
    let archive = make_asset_archive(&[("index.html", b"<h1>App</h1>")]);
    let routes: serde_json::Value =
        serde_json::from_str(test_constants::MINIAPP_DEMO1_WASM_ROUTES_JSON).unwrap();

    deploy(
        &ctx.substrate_client,
        "demo-comments",
        miniapp_manifest_with_assets(wasm_bytes, routes, archive),
    )
    .await;

    // 1. Initial GET /api/comments -> empty list []
    let (mut send, mut recv) = open_http_stream(
        ctx.substrate_client.connection().as_ref().unwrap(),
        "demo-comments",
        None,
    )
    .await;
    send.write_all(b"GET /api/comments HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
    let resp = read_full_http_response(&mut recv).await;
    assert_eq!(resp.status, 200);
    let comments: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(comments, serde_json::json!([]));

    // 2. POST /api/comments with invalid payload -> 422 Unprocessable Entity
    let (mut send_err, mut recv_err) = open_http_stream(
        ctx.substrate_client.connection().as_ref().unwrap(),
        "demo-comments",
        None,
    )
    .await;
    let post_err_body = b"{\"invalid\":123}";
    let post_err_req = format!(
        "POST /api/comments HTTP/1.1\r\nHost: localhost\r\nContent-Type: \
         application/json\r\nContent-Length: {}\r\n\r\n{}",
        post_err_body.len(),
        std::str::from_utf8(post_err_body).unwrap()
    );
    send_err.write_all(post_err_req.as_bytes()).await.unwrap();
    let resp_err = read_full_http_response(&mut recv_err).await;
    assert_eq!(resp_err.status, 422);
    assert!(String::from_utf8_lossy(&resp_err.body).contains("text is required"));

    // 3. POST /api/comments -> 201 Created
    let (mut send2, mut recv2) = open_http_stream(
        ctx.substrate_client.connection().as_ref().unwrap(),
        "demo-comments",
        None,
    )
    .await;
    let post_body = b"{\"text\":\"First comment from test\"}";
    let post_req = format!(
        "POST /api/comments HTTP/1.1\r\nHost: localhost\r\nContent-Type: \
         application/json\r\nContent-Length: {}\r\n\r\n{}",
        post_body.len(),
        std::str::from_utf8(post_body).unwrap()
    );
    send2.write_all(post_req.as_bytes()).await.unwrap();
    let resp2 = read_full_http_response(&mut recv2).await;
    assert_eq!(resp2.status, 201);

    // 4. GET /api/comments -> list containing "First comment from test"
    let (mut send3, mut recv3) = open_http_stream(
        ctx.substrate_client.connection().as_ref().unwrap(),
        "demo-comments",
        None,
    )
    .await;
    send3.write_all(b"GET /api/comments HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
    let resp3 = read_full_http_response(&mut recv3).await;
    assert_eq!(resp3.status, 200);
    let comments3: serde_json::Value = serde_json::from_slice(&resp3.body).unwrap();
    let arr = comments3.as_array().expect("expected json array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["text"], "First comment from test");
}

#[tokio::test]
async fn test_miniapp_demo1_wasm_stream_upload_and_download() {
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(23109, 23110, 23111).await;
    ctx.substrate_client.inject_kek("21".repeat(32)).await.expect("inject_kek failed");

    let wasm_bytes = std::fs::read(test_constants::miniapp_demo1_wasm_path())
        .expect("miniapp_demo1_wasm.wasm not built");
    let archive = make_asset_archive(&[("index.html", b"<h1>App</h1>")]);
    let routes: serde_json::Value =
        serde_json::from_str(test_constants::MINIAPP_DEMO1_WASM_ROUTES_JSON).unwrap();

    deploy(
        &ctx.substrate_client,
        "demo-files",
        miniapp_manifest_with_assets(wasm_bytes, routes, archive),
    )
    .await;

    // 1. Upload multi-chunk file (64 KiB > 8 chunks of 8 KiB) via POST
    //    /api/files?metadata=large_doc.bin
    let file_content = vec![0x42u8; 64 * 1024];
    let (mut send, mut recv) =
        open_http_stream(ctx.substrate_client.connection().as_ref().unwrap(), "demo-files", None)
            .await;
    let post_req = format!(
        "POST /api/files?metadata=large_doc.bin HTTP/1.1\r\nHost: localhost\r\nContent-Length: \
         {}\r\n\r\n",
        file_content.len()
    );
    send.write_all(post_req.as_bytes()).await.unwrap();
    send.write_all(&file_content).await.unwrap();
    let resp = read_full_http_response(&mut recv).await;
    assert_eq!(resp.status, 200);
    let upload_json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(upload_json["status"], "uploaded");

    // 2. Query file list via GET /api/files
    let (mut send2, mut recv2) =
        open_http_stream(ctx.substrate_client.connection().as_ref().unwrap(), "demo-files", None)
            .await;
    send2.write_all(b"GET /api/files HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
    let resp2 = read_full_http_response(&mut recv2).await;
    assert_eq!(resp2.status, 200);
    let files_json: serde_json::Value = serde_json::from_slice(&resp2.body).unwrap();
    let files_arr = files_json.as_array().expect("expected json array");
    assert_eq!(files_arr.len(), 1);
    assert_eq!(files_arr[0]["name"], "large_doc.bin");
    assert_eq!(files_arr[0]["size"], file_content.len() as u64);

    // 3. Download file via GET /api/files/large_doc.bin
    let (mut send3, mut recv3) =
        open_http_stream(ctx.substrate_client.connection().as_ref().unwrap(), "demo-files", None)
            .await;
    send3
        .write_all(b"GET /api/files/large_doc.bin HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let resp3 = read_full_http_response(&mut recv3).await;
    assert_eq!(resp3.status, 200);
    assert_eq!(resp3.body.len(), file_content.len());
    assert_eq!(resp3.body, file_content);

    // 4. Download non-existent file via GET /api/files/missing.bin -> 404 Not Found
    let (mut send4, mut recv4) =
        open_http_stream(ctx.substrate_client.connection().as_ref().unwrap(), "demo-files", None)
            .await;
    send4
        .write_all(b"GET /api/files/missing.bin HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let resp4 = read_full_http_response(&mut recv4).await;
    assert_eq!(resp4.status, 404);
}

#[tokio::test]
async fn test_miniapp_demo1_wasm_sse_live_updates() {
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(23112, 23113, 23114).await;
    ctx.substrate_client.inject_kek("21".repeat(32)).await.expect("inject_kek failed");

    let wasm_bytes = std::fs::read(test_constants::miniapp_demo1_wasm_path())
        .expect("miniapp_demo1_wasm.wasm not built");
    let archive = make_asset_archive(&[("index.html", b"<h1>App</h1>")]);
    let routes: serde_json::Value =
        serde_json::from_str(test_constants::MINIAPP_DEMO1_WASM_ROUTES_JSON).unwrap();

    deploy(
        &ctx.substrate_client,
        "demo-sse",
        miniapp_manifest_with_assets(wasm_bytes, routes, archive),
    )
    .await;

    // 1. Connect SSE client on /api/events
    let (mut sse_send, mut sse_recv) =
        open_http_stream(ctx.substrate_client.connection().as_ref().unwrap(), "demo-sse", None)
            .await;
    sse_send
        .write_all(
            b"GET /api/events HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n\r\n",
        )
        .await
        .unwrap();

    // 2. Post a comment on another stream
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (mut send_post, mut recv_post) =
        open_http_stream(ctx.substrate_client.connection().as_ref().unwrap(), "demo-sse", None)
            .await;
    let post_body = b"{\"text\":\"Trigger SSE update\"}";
    let post_req = format!(
        "POST /api/comments HTTP/1.1\r\nHost: localhost\r\nContent-Type: \
         application/json\r\nContent-Length: {}\r\n\r\n{}",
        post_body.len(),
        std::str::from_utf8(post_body).unwrap()
    );
    send_post.write_all(post_req.as_bytes()).await.unwrap();
    let post_resp = read_full_http_response(&mut recv_post).await;
    assert_eq!(post_resp.status, 201);

    // 3. Read SSE event frame from sse_recv
    let mut sse_buf = vec![0u8; 4096];
    let n_sse = timeout(Duration::from_secs(5), sse_recv.read(&mut sse_buf))
        .await
        .expect("timed out waiting for SSE frame")
        .unwrap()
        .unwrap();
    let raw_sse = String::from_utf8_lossy(&sse_buf[..n_sse]);
    assert!(raw_sse.contains("commentUpdateTimestamp"), "SSE stream received: {raw_sse}");
}

#[tokio::test]
async fn test_miniapp_demo1_wasm_websocket_echo_and_updates() {
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(23115, 23116, 23117).await;
    ctx.substrate_client.inject_kek("21".repeat(32)).await.expect("inject_kek failed");

    let wasm_bytes = std::fs::read(test_constants::miniapp_demo1_wasm_path())
        .expect("miniapp_demo1_wasm.wasm not built");
    let archive = make_asset_archive(&[("index.html", b"<h1>App</h1>")]);
    let routes: serde_json::Value =
        serde_json::from_str(test_constants::MINIAPP_DEMO1_WASM_ROUTES_JSON).unwrap();

    deploy(
        &ctx.substrate_client,
        "demo-ws",
        miniapp_manifest_with_assets(wasm_bytes, routes, archive),
    )
    .await;

    // 1. Connect WebSocket client on /ws
    let (mut ws_send, mut ws_recv) =
        open_http_stream(ctx.substrate_client.connection().as_ref().unwrap(), "demo-ws", None)
            .await;
    let upgrade_req = build_websocket_upgrade_request("/ws");
    ws_send.write_all(&upgrade_req).await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = ws_recv.read(&mut buf).await.unwrap().unwrap();
    let resp = parse_http_response(&buf[..n]);
    assert_eq!(resp.status, 101);
    assert_eq!(resp.headers.get("upgrade").map(|s| s.as_str()), Some("websocket"));

    // 2. Send text frame to echo endpoint
    let msg = b"Hello websocket!";
    let frame = build_masked_text_frame(msg);
    ws_send.write_all(&frame).await.unwrap();

    // 3. Receive unicast echo frame (binary frame: 0x82 or text frame: 0x81)
    let mut header = [0u8; 2];
    ws_recv.read_exact(&mut header).await.unwrap();
    assert!(header[0] == 0x81 || header[0] == 0x82);
    let payload_len = (header[1] & 0x7F) as usize;
    let mut echo_body = vec![0u8; payload_len];
    ws_recv.read_exact(&mut echo_body).await.unwrap();
    let echo_json: serde_json::Value = serde_json::from_slice(&echo_body).unwrap();
    assert_eq!(echo_json["recdMsg"], "Received: Hello websocket!");

    // 4. Post a comment on another stream -> WebSocket receives broadcast update
    let (mut send_post, mut recv_post) =
        open_http_stream(ctx.substrate_client.connection().as_ref().unwrap(), "demo-ws", None)
            .await;
    let post_body = b"{\"text\":\"Broadcast to WS\"}";
    let post_req = format!(
        "POST /api/comments HTTP/1.1\r\nHost: localhost\r\nContent-Type: \
         application/json\r\nContent-Length: {}\r\n\r\n{}",
        post_body.len(),
        std::str::from_utf8(post_body).unwrap()
    );
    send_post.write_all(post_req.as_bytes()).await.unwrap();
    let post_resp = read_full_http_response(&mut recv_post).await;
    assert_eq!(post_resp.status, 201);

    // Read broadcast frame from ws_recv
    let mut bcast_header = [0u8; 2];
    timeout(Duration::from_secs(5), ws_recv.read_exact(&mut bcast_header))
        .await
        .expect("timed out waiting for WS broadcast frame")
        .unwrap();
    assert!(bcast_header[0] == 0x81 || bcast_header[0] == 0x82);
    let bcast_len = (bcast_header[1] & 0x7F) as usize;
    let mut bcast_body = vec![0u8; bcast_len];
    ws_recv.read_exact(&mut bcast_body).await.unwrap();
    let bcast_json: serde_json::Value = serde_json::from_slice(&bcast_body).unwrap();
    assert!(bcast_json.get("commentUpdateTimestamp").is_some());
}
