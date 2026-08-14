#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! M06A A2 end-to-end tests: the guest HTTP route target
//! (`syneroym:http/incoming-handler#handle-request`), driven by hand-built
//! raw HTTP/1.1 request/response bytes over a real Iroh QUIC bidi stream --
//! the same harness `static_assets_e2e.rs` (M06A A1) and
//! `http_passthrough_e2e.rs` (M3B Slice 7) use. Helpers are duplicated
//! rather than shared across independent `tests/*.rs` binaries, matching
//! this workspace's existing convention.

use std::{collections::HashMap, time::Duration};

use httparse::{EMPTY_HEADER, Response as HttparseResponse, Status};
use iroh::endpoint::{RecvStream, SendStream};
use rustls::crypto::ring;
use syneroym_core::{
    config::AppSandboxRole,
    dht_registry::{EndpointInfo, EndpointMechanism, EndpointType},
    test_constants,
    util::short_hash,
};
use syneroym_identity::{Identity, substrate};
use syneroym_observability::MemoryRecorder;
use syneroym_router::{RoutePreamble, RouteProtocol, RouteTransport};
use syneroym_sdk::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, SyneroymClient,
    TransportConnection, WasmManifest,
};
use tokio::{
    io::{AsyncReadExt as TokioAsyncReadExt, AsyncWriteExt as TokioAsyncWriteExt},
    net::TcpStream,
    time::timeout,
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
            interfaces: vec![test_constants::HTTP_GUEST_TEST_DRIVER_INTERFACE.to_string()],
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

/// Opens a fresh Iroh bidi stream and writes the `http://http-native|
/// <service_id>` route preamble. `pubkey` controls whether this connection
/// is genuinely anonymous (`None`, the direct-WebRTC shape F5a describes)
/// or self-asserted (`Some`, an ephemeral identity with no delegation --
/// mirrors what every other e2e file in this workspace does for an
/// ordinary "not anonymous" connection).
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

async fn http_request(
    conn: &TransportConnection,
    service_id: &str,
    pubkey: Option<&Identity>,
    method: &str,
    path_and_query: &str,
    body: &[u8],
) -> HttpResponse {
    let (mut send, mut recv) = open_http_stream(conn, service_id, pubkey).await;
    let mut request = format!(
        "{method} {path_and_query} HTTP/1.1\r\nHost: localhost\r\nConnection: \
         close\r\nContent-Length: {}\r\n",
        body.len()
    );
    request.push_str("\r\n");
    send.write_all(request.as_bytes()).await.expect("write request head failed");
    if !body.is_empty() {
        // Tolerated, not asserted: a server that rejects the request before
        // the body finishes (e.g. an over-cap body, M06A D-A2-8) stops
        // reading and the stream write fails -- the response is still read
        // below, exactly like a real HTTP client that gets an early
        // response while still uploading.
        let _ = send.write_all(body).await;
    }
    let _ = send.finish();

    let raw = recv.read_to_end(64 * 1024 * 1024).await.expect("read response failed");
    parse_http_response(&raw)
}

fn connect_peer(app_service_id: &str, mechanisms: &[EndpointMechanism]) -> SyneroymClient {
    SyneroymClient::new_with_mechanisms(app_service_id.to_string(), mechanisms.to_vec())
        .with_registry_dht(false)
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

fn read_http_guest_test_wasm() -> Option<Vec<u8>> {
    std::fs::read(test_constants::http_guest_test_wasm_path()).ok()
}

macro_rules! skip_if_missing {
    ($test_name:literal) => {
        match read_http_guest_test_wasm() {
            Some(bytes) => bytes,
            None => {
                eprintln!(
                    "Skipping {}: http-guest-test WASM artifact not found (build \
                     test-components/http-guest-test with `cargo component build --release \
                     --target wasm32-wasip2`)",
                    $test_name
                );
                return;
            }
        }
    };
}

// ---------------------------------------------------------------------
// D-A2-7: a non-public route rejects an anonymous caller before the
// component is instantiated
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_anonymous_request_to_non_public_route_is_401_with_zero_instantiations() {
    let wasm_bytes = skip_if_missing!("test_anonymous_request_to_non_public_route_is_401");
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(9200, 9201, 9202).await;
    ctx.substrate_client.inject_kek("30".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let routes = serde_json::json!({"http_routes": [
        {"method": "GET", "path": "/whoami", "target": "guest", "operation": "handle-request", "public": false}
    ]});
    deploy(&ctx.substrate_client, &app_service_id, guest_wasm_manifest(wasm_bytes, routes)).await;

    let mut peer = connect_peer(&app_service_id, &ctx.substrate_mechanisms);
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");

    let before = counter_value("substrate.wasm.instantiations_total");
    let resp = http_request(&conn, &app_service_id, None, "GET", "/whoami", &[]).await;
    let after = counter_value("substrate.wasm.instantiations_total");
    assert_eq!(resp.status, 401);
    assert_eq!(after, before, "a 401 must never instantiate the component");

    let _ = peer.shutdown().await;
    ctx.teardown().await;
}

// ---------------------------------------------------------------------
// D-A2-7: a public route reaches the guest with no verified caller
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_public_route_reaches_guest_and_whoami_reports_anonymous() {
    let wasm_bytes =
        skip_if_missing!("test_public_route_reaches_guest_and_whoami_reports_anonymous");
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(9203, 9204, 9205).await;
    ctx.substrate_client.inject_kek("31".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let routes = serde_json::json!({"http_routes": [
        {"method": "GET", "path": "/whoami", "target": "guest", "operation": "handle-request", "public": true}
    ]});
    deploy(&ctx.substrate_client, &app_service_id, guest_wasm_manifest(wasm_bytes, routes)).await;

    let mut peer = connect_peer(&app_service_id, &ctx.substrate_mechanisms);
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");

    let resp = http_request(&conn, &app_service_id, None, "GET", "/whoami", &[]).await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, b"anonymous");

    let _ = peer.shutdown().await;
    ctx.teardown().await;
}

// ---------------------------------------------------------------------
// F5a: the client gateway proxies under the node's own DID, so a
// non-public route is reached anyway -- pinned so this limitation cannot
// quietly stop being true or be mistaken for authentication.
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_through_the_gateway_a_non_public_route_is_reached_and_reports_self_asserted_node_did()
{
    let wasm_bytes = skip_if_missing!("test_through_the_gateway_reports_self_asserted_node_did");
    let _ = ring::default_provider().install_default();
    let registry_port = 9207u16;
    let gateway_port = 9208u16;
    let ctx = SubstrateTestContext::setup(9206, registry_port, gateway_port).await;
    ctx.substrate_client.inject_kek("32".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let routes = serde_json::json!({"http_routes": [
        {"method": "GET", "path": "/whoami", "target": "guest", "operation": "handle-request", "public": false}
    ]});
    deploy(&ctx.substrate_client, &app_service_id, guest_wasm_manifest(wasm_bytes, routes)).await;

    // Publish the app's own endpoint record so the gateway's unscoped
    // `s<hash>.localhost` host form (ADR-0022 §7) can resolve it -- direct
    // HTTP POST to the registry, independent of `deploy`'s own
    // `registry_certificate` path (M3B's `basic_lifecycle.rs` does the
    // same for the same reason).
    let registry_url = format!("http://localhost:{registry_port}");
    let info = EndpointInfo {
        service_id: app_service_id.clone(),
        substrate_id: ctx.substrate_client.service_id().to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: ctx.substrate_mechanisms.clone(),
        nickname: None,
        is_private: false,
        ttl: None,
        not_after: u64::MAX / 2,
        generation: 0,
    };
    let signed = info.sign(&app_identity).expect("failed to sign endpoint info");
    let http = reqwest::Client::new();
    let res = http
        .post(format!("{registry_url}/register"))
        .json(&signed)
        .send()
        .await
        .expect("failed to register app in the HTTP registry");
    assert!(res.status().is_success(), "registry registration failed: {:?}", res.text().await);

    let node_did = ctx.substrate_client.service_id().to_string();
    let host = format!("s{}.localhost:{gateway_port}", short_hash(&app_service_id));
    let request = format!(
        "GET /whoami HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    );
    let mut stream =
        timeout(Duration::from_secs(10), TcpStream::connect(("127.0.0.1", gateway_port)))
            .await
            .expect("connecting to the client gateway timed out")
            .expect("failed to connect to the client gateway");
    stream.write_all(request.as_bytes()).await.expect("failed to write gateway request");
    let mut raw = Vec::new();
    timeout(Duration::from_secs(10), stream.read_to_end(&mut raw))
        .await
        .expect("reading the gateway response timed out")
        .expect("failed to read gateway response");
    let resp = parse_http_response(&raw);

    assert_eq!(
        resp.status, 200,
        "the 401 gate must not fire through the gateway -- it always presents a usable pubkey"
    );
    assert_eq!(
        resp.body,
        format!("self-asserted:{node_did}").into_bytes(),
        "the gateway proxies under the node's OWN DID, identical for every visitor -- not an end \
         user's identity"
    );

    ctx.teardown().await;
}

// ---------------------------------------------------------------------
// Exit criterion 5: the guest's own rejection status and message
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_reject_returns_the_guests_own_status_and_message() {
    let wasm_bytes = skip_if_missing!("test_reject_returns_the_guests_own_status_and_message");
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(9209, 9210, 9211).await;
    ctx.substrate_client.inject_kek("33".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let routes = serde_json::json!({"http_routes": [
        {"method": "POST", "path": "/reject", "target": "guest", "operation": "handle-request", "public": false}
    ]});
    deploy(&ctx.substrate_client, &app_service_id, guest_wasm_manifest(wasm_bytes, routes)).await;

    let mut peer = connect_peer(&app_service_id, &ctx.substrate_mechanisms);
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");
    let asserting = Identity::generate().unwrap();

    let resp = http_request(&conn, &app_service_id, Some(&asserting), "POST", "/reject", &[]).await;
    assert_eq!(resp.status, 422);
    assert_eq!(resp.body, b"comment is empty");

    let _ = peer.shutdown().await;
    ctx.teardown().await;
}

// ---------------------------------------------------------------------
// D-A2-8: an over-cap request body is rejected before instantiation
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_over_cap_request_body_is_413_with_zero_instantiations() {
    let wasm_bytes = skip_if_missing!("test_over_cap_request_body_is_413");
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(9212, 9213, 9214).await;
    ctx.substrate_client.inject_kek("34".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let routes = serde_json::json!({"http_routes": [
        {"method": "POST", "path": "/echo", "target": "guest", "operation": "handle-request", "public": false}
    ]});
    deploy(&ctx.substrate_client, &app_service_id, guest_wasm_manifest(wasm_bytes, routes)).await;

    let mut peer = connect_peer(&app_service_id, &ctx.substrate_mechanisms);
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");
    let asserting = Identity::generate().unwrap();

    let oversized = vec![0u8; 2 * 1024 * 1024];
    let before = counter_value("substrate.wasm.instantiations_total");
    let resp =
        http_request(&conn, &app_service_id, Some(&asserting), "POST", "/echo", &oversized).await;
    let after = counter_value("substrate.wasm.instantiations_total");
    assert_eq!(resp.status, 413);
    assert_eq!(after, before, "an over-cap body must never instantiate the component");

    let _ = peer.shutdown().await;
    ctx.teardown().await;
}

// ---------------------------------------------------------------------
// Matrix row 5 (wasm-execution half): a trap or a wasm-execution loop each
// answer 500 with a structured error, and the connection stays usable.
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_trap_and_spin_return_500_and_a_new_stream_still_succeeds() {
    let wasm_bytes = skip_if_missing!("test_trap_and_spin_return_500");
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(9215, 9216, 9217).await;
    ctx.substrate_client.inject_kek("35".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let routes = serde_json::json!({"http_routes": [
        {"method": "GET", "path": "/trap", "target": "guest", "operation": "handle-request", "public": false},
        {"method": "GET", "path": "/spin", "target": "guest", "operation": "handle-request", "public": false},
        {"method": "GET", "path": "/whoami", "target": "guest", "operation": "handle-request", "public": false}
    ]});
    deploy(&ctx.substrate_client, &app_service_id, guest_wasm_manifest(wasm_bytes, routes)).await;

    let mut peer = connect_peer(&app_service_id, &ctx.substrate_mechanisms);
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");
    let asserting = Identity::generate().unwrap();

    let trap_resp =
        http_request(&conn, &app_service_id, Some(&asserting), "GET", "/trap", &[]).await;
    assert_eq!(trap_resp.status, 500);

    // Bounded by the dispatch epoch deadline (default 5s) -- the guest
    // never returns on its own.
    let spin_resp = timeout(
        Duration::from_secs(15),
        http_request(&conn, &app_service_id, Some(&asserting), "GET", "/spin", &[]),
    )
    .await
    .expect("the epoch deadline must bound a guest wasm-execution loop");
    assert_eq!(spin_resp.status, 500);

    // A fresh stream (this file's `open_http_stream` opens a new bidi
    // stream and closes it per call) still succeeds -- the connection
    // itself was never poisoned by either failure.
    let ok_resp =
        http_request(&conn, &app_service_id, Some(&asserting), "GET", "/whoami", &[]).await;
    assert_eq!(ok_resp.status, 200);

    let _ = peer.shutdown().await;
    ctx.teardown().await;
}

// ---------------------------------------------------------------------
// Matrix row 6: an oversized or malformed guest response is bounded and
// rejected, never streamed to the client.
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_huge_and_bad_header_return_500_with_no_partial_body() {
    let wasm_bytes = skip_if_missing!("test_huge_and_bad_header_return_500");
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(9218, 9219, 9220).await;
    ctx.substrate_client.inject_kek("36".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let routes = serde_json::json!({"http_routes": [
        {"method": "GET", "path": "/huge", "target": "guest", "operation": "handle-request", "public": false},
        {"method": "GET", "path": "/bad-header", "target": "guest", "operation": "handle-request", "public": false}
    ]});
    deploy(&ctx.substrate_client, &app_service_id, guest_wasm_manifest(wasm_bytes, routes)).await;

    let mut peer = connect_peer(&app_service_id, &ctx.substrate_mechanisms);
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");
    let asserting = Identity::generate().unwrap();

    let huge_resp =
        http_request(&conn, &app_service_id, Some(&asserting), "GET", "/huge", &[]).await;
    assert_eq!(huge_resp.status, 500);
    assert!(
        huge_resp.body.len() < 4096,
        "an oversized guest response must be rejected, not partially streamed: got {} bytes",
        huge_resp.body.len()
    );

    let bad_header_resp =
        http_request(&conn, &app_service_id, Some(&asserting), "GET", "/bad-header", &[]).await;
    assert_eq!(bad_header_resp.status, 500);

    let _ = peer.shutdown().await;
    ctx.teardown().await;
}

// ---------------------------------------------------------------------
// D-A2-11: guest HTTP concurrency is bounded per service, and exhausting
// it degrades that service (503 + Retry-After), not a 500 or a hang.
// ---------------------------------------------------------------------

// Multi-threaded on purpose: the guest's busy-spin `/slow` handler has no
// host-import call inside it to yield on, so a current-thread runtime would
// let it monopolize the only worker until it returns, defeating the very
// concurrency this test needs to observe.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_guest_http_concurrency_limit_returns_503_with_retry_after() {
    let wasm_bytes = skip_if_missing!("test_guest_http_concurrency_limit_returns_503");
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup_with(9221, 9222, 9223, |config| {
        config.roles.app_sandbox =
            Some(AppSandboxRole { max_concurrent_guest_http_per_service: 1, ..Default::default() });
    })
    .await;
    ctx.substrate_client.inject_kek("37".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let routes = serde_json::json!({"http_routes": [
        {"method": "GET", "path": "/slow", "target": "guest", "operation": "handle-request", "public": false}
    ]});
    deploy(&ctx.substrate_client, &app_service_id, guest_wasm_manifest(wasm_bytes, routes)).await;

    let mut peer = connect_peer(&app_service_id, &ctx.substrate_mechanisms);
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");
    let asserting = Identity::generate().unwrap();

    // Budget of 1: the first request holds the only permit for 6s, far past
    // `GUEST_HTTP_ADMISSION_TIMEOUT` (2s, fixed), so a second request fired
    // while the first is still in flight must time out waiting for
    // admission and get 503, not queue past the fixed wait or 500.
    let conn_a = conn.clone();
    let service_id_a = app_service_id.clone();
    let asserting_a = Identity::generate().unwrap();
    let first = tokio::spawn(async move {
        http_request(&conn_a, &service_id_a, Some(&asserting_a), "GET", "/slow?ms=4000", &[]).await
    });
    // Give the first request time to actually acquire the sole permit
    // before the second one is sent.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let second =
        http_request(&conn, &app_service_id, Some(&asserting), "GET", "/slow?ms=4000", &[]).await;

    assert_eq!(
        second.status, 503,
        "a request that cannot get an admission permit within the fixed timeout must be 503, not \
         500 or a hang"
    );
    assert_eq!(second.headers.get("retry-after").map(String::as_str), Some("1"));

    let first_resp = first.await.expect("first request task panicked");
    assert_eq!(first_resp.status, 200, "the request that held the permit must still succeed");

    let _ = peer.shutdown().await;
    ctx.teardown().await;
}

// ---------------------------------------------------------------------
// A guest route and other bridge targets/A1 assets coexist without either
// shadowing the other.
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_guest_route_and_data_layer_route_coexist() {
    let wasm_bytes = skip_if_missing!("test_guest_route_and_data_layer_route_coexist");
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(9224, 9225, 9226).await;
    ctx.substrate_client.inject_kek("26".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let routes = serde_json::json!({"http_routes": [
        {"method": "GET", "path": "/whoami", "target": "guest", "operation": "handle-request", "public": false},
        {"method": "GET", "path": "/items/{id}", "target": "data-layer", "operation": "get",
         "collection": "items"}
    ]});
    deploy(&ctx.substrate_client, &app_service_id, guest_wasm_manifest(wasm_bytes, routes)).await;

    let mut peer = connect_peer(&app_service_id, &ctx.substrate_mechanisms);
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");
    let asserting = Identity::generate().unwrap();

    let guest_resp =
        http_request(&conn, &app_service_id, Some(&asserting), "GET", "/whoami", &[]).await;
    assert_eq!(guest_resp.status, 200);
    assert!(guest_resp.body.starts_with(b"self-asserted:"));

    // `items` was never created, so `data-layer::get` legitimately answers
    // its own "collection not found" -- proving the request reached the
    // native-dispatch bridge, not the guest handler (which would answer
    // "no such test path" instead).
    let route_resp =
        http_request(&conn, &app_service_id, Some(&asserting), "GET", "/items/abc", &[]).await;
    let body: serde_json::Value = serde_json::from_slice(&route_resp.body).unwrap();
    assert_eq!(body["error"]["code"], serde_json::json!(-32011));

    let _ = peer.shutdown().await;
    ctx.teardown().await;
}
