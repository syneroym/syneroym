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
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};
use syneroym_sdk::{
    ArtifactSource, AssetBundle, DeployManifest, ServiceConfig, ServiceType, SyneroymClient,
    TransportConnection, Visibility, WasmManifest,
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
            visibility: None,
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

fn guest_wasm_manifest_with_assets(
    wasm_bytes: Vec<u8>,
    http_routes: serde_json::Value,
    archive: Vec<u8>,
) -> DeployManifest {
    let mut manifest = guest_wasm_manifest(wasm_bytes, http_routes);
    manifest.config.assets = Some(AssetBundle {
        archive: ArtifactSource::Binary(archive),
        hash: None,
        visibility: Some(Visibility::Public),
    });
    manifest
}

/// Same construction `static_assets_e2e.rs` (M06A A1) uses for its own
/// fixture archives.
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

/// F5b's attacker-controlled case: a UCAN token is attached, but it is not
/// rooted at anything this node trusts, so `build_caller` fail-opens to
/// dropping its capabilities rather than rejecting the connection. Same
/// preamble as `open_http_stream` otherwise -- no `delegation`, so
/// `D-A2-12` has nothing to report `delegated` from either.
async fn open_http_stream_with_ucan(
    conn: &TransportConnection,
    service_id: &str,
    pubkey: &Identity,
    ucan: CapabilityToken,
) -> (SendStream, RecvStream) {
    let TransportConnection::Iroh { conn, .. } = conn;
    let (mut send, recv) = conn.open_bi().await.expect("open_bi failed");
    let preamble = RoutePreamble {
        transport: RouteTransport::Http,
        protocol: RouteProtocol::JsonRpc,
        interface: "http-native".to_string(),
        service_id: service_id.to_string(),
        enc: None,
        pubkey: Some(hex::encode(pubkey.public_key().to_bytes())),
        delegation: None,
        ucan: Some(ucan),
        dir: None,
    };
    send.write_all(preamble.to_preamble_line().as_bytes()).await.expect("write preamble failed");
    (send, recv)
}

async fn http_request_with_ucan(
    conn: &TransportConnection,
    service_id: &str,
    pubkey: &Identity,
    ucan: CapabilityToken,
    method: &str,
    path_and_query: &str,
) -> HttpResponse {
    let (mut send, mut recv) = open_http_stream_with_ucan(conn, service_id, pubkey, ucan).await;
    let request = format!(
        "{method} {path_and_query} HTTP/1.1\r\nHost: localhost\r\nConnection: \
         close\r\nContent-Length: 0\r\n\r\n"
    );
    send.write_all(request.as_bytes()).await.expect("write request head failed");
    let _ = send.finish();
    let raw = recv.read_to_end(64 * 1024 * 1024).await.expect("read response failed");
    parse_http_response(&raw)
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
        read_http_guest_test_wasm()
            .unwrap_or_else(|| panic!("WASM artifact not found for {}", $test_name))
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
        config.roles.app_sandbox = Some(AppSandboxRole {
            max_concurrent_guest_http_per_service: 1,
            // The default (5s) is a *tighter* ceiling than this test's own
            // busy-spin needs and traps the guest call with "wasm trap:
            // interrupt" before it can hold the permit long enough -- raised
            // here, not lowered elsewhere, since the default protects the
            // hot dispatch path this test does not represent.
            dispatch_epoch_timeout_secs: 15,
            ..Default::default()
        });
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

    // Budget of 1: the first request holds the only permit for 8s -- within
    // the 15s `dispatch_epoch_timeout_secs` raised above, and far past
    // `GUEST_HTTP_ADMISSION_TIMEOUT` (2s, fixed) plus the buffer below -- so
    // a second request fired while the first is still in flight must time
    // out waiting for admission and get 503, not queue past the fixed wait
    // or 500.
    let conn_a = conn.clone();
    let service_id_a = app_service_id.clone();
    let asserting_a = Identity::generate().unwrap();
    let first = tokio::spawn(async move {
        http_request(&conn_a, &service_id_a, Some(&asserting_a), "GET", "/slow?ms=8000", &[]).await
    });
    // Give the first request time to actually acquire the sole permit
    // before the second one is sent. This has to outlast the QUIC
    // stream-open + request-parse + dispatch path landing the *first*
    // request at the semaphore, not just the semaphore acquire itself --
    // under CI's shared, contended runners that whole path can take well
    // over the 500ms this used to budget, letting the second request win
    // the race for the sole permit and return 200 where 503 was expected.
    // 2s leaves 6s of headroom before the first request's own 8s hold ends,
    // comfortably longer than `GUEST_HTTP_ADMISSION_TIMEOUT` (2s) so the
    // second request's own wait still resolves to a timeout well before
    // the first request would free the permit on its own.
    tokio::time::sleep(Duration::from_millis(2000)).await;
    let second =
        http_request(&conn, &app_service_id, Some(&asserting), "GET", "/slow?ms=8000", &[]).await;

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
// D-A2-4: the path-param name the host sends and the value `match_path`
// captured describe the same segment, over the real wire.
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_items_path_param_matches_the_captured_url_segment() {
    let wasm_bytes = skip_if_missing!("test_items_path_param_matches_the_captured_url_segment");
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(9227, 9228, 9229).await;
    ctx.substrate_client.inject_kek("38".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let routes = serde_json::json!({"http_routes": [
        {"method": "GET", "path": "/items/{id}", "target": "guest", "operation": "handle-request", "public": false}
    ]});
    deploy(&ctx.substrate_client, &app_service_id, guest_wasm_manifest(wasm_bytes, routes)).await;

    let mut peer = connect_peer(&app_service_id, &ctx.substrate_mechanisms);
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");
    let asserting = Identity::generate().unwrap();

    let resp =
        http_request(&conn, &app_service_id, Some(&asserting), "GET", "/items/42", &[]).await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, b"42", "the id the guest echoed must be the id in the URL");

    let _ = peer.shutdown().await;
    ctx.teardown().await;
}

// ---------------------------------------------------------------------
// D-A2-5: framing headers the guest sets are stripped, and `Content-Length`
// is always the host's, over the real wire.
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_framing_headers_are_stripped_and_content_length_is_the_hosts() {
    let wasm_bytes =
        skip_if_missing!("test_framing_headers_are_stripped_and_content_length_is_the_hosts");
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(9230, 9231, 9232).await;
    ctx.substrate_client.inject_kek("39".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let routes = serde_json::json!({"http_routes": [
        {"method": "GET", "path": "/framing", "target": "guest", "operation": "handle-request", "public": false}
    ]});
    deploy(&ctx.substrate_client, &app_service_id, guest_wasm_manifest(wasm_bytes, routes)).await;

    let mut peer = connect_peer(&app_service_id, &ctx.substrate_mechanisms);
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");
    let asserting = Identity::generate().unwrap();

    let resp = http_request(&conn, &app_service_id, Some(&asserting), "GET", "/framing", &[]).await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, b"ok");
    // The guest set `content-length: 999`, which would desync the response
    // if it survived; the host strips it and computes its own from the real
    // body. (`connection` is also stripped from the guest, but the test
    // client sends `Connection: close` on its own request, so the server's
    // own `Connection` header on the response isn't decisive proof either
    // way -- `content-length` is the one value nothing else in the stack
    // would coincidentally produce.)
    assert_eq!(resp.headers.get("content-length").map(String::as_str), Some("2"));

    let _ = peer.shutdown().await;
    ctx.teardown().await;
}

// ---------------------------------------------------------------------
// D-A2-12/F5b: a UCAN attacker-controlled and rooted at nothing this node
// trusts must never be reported to the guest as `ucan:...` -- `build_caller`
// fails open to dropping its capabilities, and `guest_caller_identity` must
// carry that failure through rather than trusting `preamble.ucan.is_some()`
// alone. Unit-tested against `guest_caller_identity` and `build_caller`
// directly already; this is the one wire-level proof that the two compose
// correctly end to end.
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_rejected_ucan_reports_self_asserted_not_ucan() {
    let wasm_bytes = skip_if_missing!("test_rejected_ucan_reports_self_asserted_not_ucan");
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(9239, 9240, 9241).await;
    ctx.substrate_client.inject_kek("3c".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let routes = serde_json::json!({"http_routes": [
        {"method": "GET", "path": "/whoami", "target": "guest", "operation": "handle-request", "public": false}
    ]});
    deploy(&ctx.substrate_client, &app_service_id, guest_wasm_manifest(wasm_bytes, routes)).await;

    let mut peer = connect_peer(&app_service_id, &ctx.substrate_mechanisms);
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");

    // A self-asserted connection carrying a UCAN self-issued by an attacker
    // identity that is nobody's admin root and owns nothing this node knows
    // about -- exactly the token any caller can mint for themselves.
    let asserting = Identity::generate().unwrap();
    let asserting_did = substrate::derive_did_key(&asserting.public_key());
    let attacker = Identity::generate().unwrap();
    let token = CapabilityToken::issue(
        &attacker,
        &asserting_did,
        vec![Capability {
            with: ResourceUri::service(&app_service_id, &app_service_id),
            can: Ability(Ability::DATA_LAYER_ADMIN.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .expect("issue attacker-controlled UCAN");

    let resp =
        http_request_with_ucan(&conn, &app_service_id, &asserting, token, "GET", "/whoami").await;
    assert_eq!(resp.status, 200);
    assert_eq!(
        resp.body,
        format!("self-asserted:{asserting_did}").into_bytes(),
        "an untrusted UCAN must never be reported to the guest as a `ucan:` caller"
    );

    let _ = peer.shutdown().await;
    ctx.teardown().await;
}

// The companion to the test above: within budget, concurrent requests queue
// for a permit and all succeed -- the 503 case above is what happens when a
// request outwaits the fixed admission timeout, not what queuing itself
// looks like.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_guest_http_requests_within_budget_all_succeed_via_queuing() {
    let wasm_bytes =
        skip_if_missing!("test_guest_http_requests_within_budget_all_succeed_via_queuing");
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup_with(9236, 9237, 9238, |config| {
        config.roles.app_sandbox =
            Some(AppSandboxRole { max_concurrent_guest_http_per_service: 2, ..Default::default() });
    })
    .await;
    ctx.substrate_client.inject_kek("3b".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let routes = serde_json::json!({"http_routes": [
        {"method": "GET", "path": "/slow", "target": "guest", "operation": "handle-request", "public": false}
    ]});
    deploy(&ctx.substrate_client, &app_service_id, guest_wasm_manifest(wasm_bytes, routes)).await;

    let mut peer = connect_peer(&app_service_id, &ctx.substrate_mechanisms);
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");

    // Budget of 2, 4 concurrent callers each holding a permit for ~300ms:
    // the 2 that don't get a permit immediately wait well under the fixed
    // 2s admission timeout, so every one of the 4 must still land 200, not
    // queue into a 503 or a 500.
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let conn = conn.clone();
        let service_id = app_service_id.clone();
        let asserting = Identity::generate().unwrap();
        tasks.push(tokio::spawn(async move {
            http_request(&conn, &service_id, Some(&asserting), "GET", "/slow?ms=300", &[]).await
        }));
    }
    for task in tasks {
        let resp = task.await.expect("guest HTTP request task panicked");
        assert_eq!(
            resp.status, 200,
            "a request within the concurrency budget's queuing capacity must succeed, not 503/500"
        );
    }

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

// A1's own coexistence test (`test_static_assets_and_http_routes_coexist`)
// pairs an asset bundle with a `data-layer` route on a plain `greeter`
// component. That never exercises a component deployed as *both* an asset
// bundle and a guest HTTP handler at once, which is the combination this
// test covers.
#[tokio::test]
async fn test_guest_route_and_asset_bundle_coexist() {
    let wasm_bytes = skip_if_missing!("test_guest_route_and_asset_bundle_coexist");
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(9233, 9234, 9235).await;
    ctx.substrate_client.inject_kek("3a".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let routes = serde_json::json!({"http_routes": [
        {"method": "GET", "path": "/whoami", "target": "guest", "operation": "handle-request", "public": true}
    ]});
    let archive = make_asset_archive(&[("index.html", b"<html>hi</html>")]);
    deploy(
        &ctx.substrate_client,
        &app_service_id,
        guest_wasm_manifest_with_assets(wasm_bytes, routes, archive),
    )
    .await;

    let mut peer = connect_peer(&app_service_id, &ctx.substrate_mechanisms);
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");

    // The asset bundle still resolves.
    let asset_resp = http_request(&conn, &app_service_id, None, "GET", "/", &[]).await;
    assert_eq!(asset_resp.status, 200);
    assert_eq!(asset_resp.body, b"<html>hi</html>");

    // The declared guest route still reaches the guest handler, not the
    // asset table.
    let guest_resp = http_request(&conn, &app_service_id, None, "GET", "/whoami", &[]).await;
    assert_eq!(guest_resp.status, 200);
    assert_eq!(guest_resp.body, b"anonymous");

    let _ = peer.shutdown().await;
    ctx.teardown().await;
}
