#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! M06A A1 end-to-end test: blob-backed static asset serving, driven by
//! hand-built raw HTTP/1.1 request/response bytes over a real Iroh QUIC
//! bidi stream -- the same harness `http_passthrough_e2e.rs` uses for M3B
//! Slice 7's HTTP passthrough (see that file's module doc for the pattern's
//! rationale). Helpers are duplicated rather than shared across the two
//! files' independent test binaries, matching this workspace's existing
//! convention for `tests/*.rs` (`wasm_deploy_manifest`/`tcp_deploy_manifest`
//! are already duplicated the same way across several e2e files).

use std::collections::HashMap;

use httparse::{EMPTY_HEADER, Response as HttparseResponse, Status};
use iroh::endpoint::{RecvStream, SendStream};
use rustls::crypto::ring;
use syneroym_core::{dht_registry::EndpointMechanism, test_constants};
use syneroym_identity::{Identity, substrate};
use syneroym_observability::MemoryRecorder;
use syneroym_router::{RoutePreamble, RouteProtocol, RouteTransport};
use syneroym_sdk::{
    ArtifactSource, AssetBundle, DeployManifest, ServiceConfig, ServiceType, SyneroymClient,
    TransportConnection, Visibility, WasmManifest,
};

mod common;
use common::SubstrateTestContext;

fn wasm_asset_manifest(
    wasm_bytes: Vec<u8>,
    archive: Vec<u8>,
    visibility: Visibility,
    custom_config: Option<serde_json::Value>,
) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: custom_config.map(|v| v.to_string()),
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: Some(AssetBundle {
                archive: ArtifactSource::Binary(archive),
                hash: None,
                visibility: Some(visibility),
            }),
        },
        service_type: ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(wasm_bytes),
            hash: None,
            interfaces: vec![],
        }),
        registry_certificate: None,
        instance_certificate: None,
    }
}

fn wasm_manifest_without_assets(wasm_bytes: Vec<u8>) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
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
            interfaces: vec![],
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

/// A minimal gzip-compressed tar archive, one entry per `(path, bytes)`
/// pair -- the same shape `syneroym_control_plane::assets`' own unit tests
/// build.
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

struct HttpResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

async fn open_http_stream(
    conn: &TransportConnection,
    service_id: &str,
) -> (SendStream, RecvStream) {
    let TransportConnection::Iroh { conn, .. } = conn;
    let (mut send, recv) = conn.open_bi().await.expect("open_bi failed");
    let identity = Identity::generate().expect("failed to generate test identity");
    let preamble = RoutePreamble {
        transport: RouteTransport::Http,
        protocol: RouteProtocol::JsonRpc,
        interface: "http-native".to_string(),
        service_id: service_id.to_string(),
        enc: None,
        pubkey: Some(hex::encode(identity.public_key().to_bytes())),
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
    method: &str,
    path_and_query: &str,
    extra_headers: &[(&str, &str)],
) -> HttpResponse {
    let (mut send, mut recv) = open_http_stream(conn, service_id).await;
    let mut request =
        format!("{method} {path_and_query} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    for (k, v) in extra_headers {
        request.push_str(&format!("{k}: {v}\r\n"));
    }
    request.push_str("\r\n");
    send.write_all(request.as_bytes()).await.expect("write request head failed");
    let _ = send.finish();

    let raw = recv.read_to_end(64 * 1024 * 1024).await.expect("read response failed");
    parse_http_response(&raw)
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

fn connect_peer(app_service_id: &str, mechanisms: &[EndpointMechanism]) -> SyneroymClient {
    SyneroymClient::new_with_mechanisms(app_service_id.to_string(), mechanisms.to_vec())
        .with_registry_dht(false)
}

/// Reads the current value of a `MemoryRecorder`-backed counter, treating
/// an absent counter (never incremented yet) as `0` -- the substrate under
/// test installs the global recorder in `ObservabilityEngine::init`, and
/// `SubstrateTestContext::setup` runs it in this same process, so the test
/// can read it directly with no metrics endpoint involved.
///
/// This is a process-wide `OnceLock` (`syneroym_observability::recorder::
/// GLOBAL_RECORDER`), reused as-is by every `SubstrateTestContext` in this
/// binary rather than reset per test -- a `before`/`after` delta read
/// around one request is only safe as long as no *other* test's substrate
/// is concurrently live to increment it in between. That is currently true
/// only because `common::SUBSTRATE_TEST_LOCK`'s guard is held for a whole
/// `SubstrateTestContext`'s lifetime (from `setup` returning to
/// `teardown`/drop, not just the setup race it was written for), so this
/// file's five tests never actually overlap. If that lock is ever narrowed
/// back to just the setup race, this delta becomes flaky the same way a
/// missing lock would: prefer `AppSandboxEngine::instantiations()` (a
/// per-engine, not process-global, counter -- see M06A D-A1-7) if that
/// coupling ever needs to go away.
fn counter_value(name: &str) -> u64 {
    MemoryRecorder::global()
        .expect("global MemoryRecorder must be installed by the substrate under test")
        .snapshot()
        .counters
        .get(name)
        .copied()
        .unwrap_or(0)
}

// ---------------------------------------------------------------------
// Exit criteria 3/4, D-A1-11, HEAD, Cache-Control
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_static_asset_serving_index_etag_and_directory_rewrite() {
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(9100, 9101, 9102).await;
    ctx.substrate_client.inject_kek("21".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let wasm_bytes = std::fs::read(test_constants::greeter_wasm_path()).unwrap();
    let archive = make_asset_archive(&[
        ("index.html", b"<html>root</html>"),
        ("sub/index.html", b"<html>sub</html>"),
        ("assets/app.js", b"console.log(1)"),
    ]);
    deploy(
        &ctx.substrate_client,
        &app_service_id,
        wasm_asset_manifest(wasm_bytes, archive, Visibility::Public, None),
    )
    .await;

    let mut peer = connect_peer(&app_service_id, &ctx.substrate_mechanisms);
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");

    // Exit criterion 3: GET / -> index.html, correct content type, and the
    // sandbox is never instantiated for it.
    let before = counter_value("substrate.wasm.instantiations_total");
    let resp = http_request(&conn, &app_service_id, "GET", "/", &[]).await;
    let after = counter_value("substrate.wasm.instantiations_total");
    assert_eq!(resp.status, 200, "GET / must resolve to index.html");
    assert_eq!(resp.body, b"<html>root</html>");
    assert_eq!(
        resp.headers.get("content-type").map(String::as_str),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(resp.headers.get("cache-control").map(String::as_str), Some("no-cache"));
    assert_eq!(after, before, "static asset GET must not instantiate the component");
    let etag = resp.headers.get("etag").cloned().expect("index.html response must carry an ETag");

    // D-A1-11: a directory index fires per-subdirectory too.
    let sub = http_request(&conn, &app_service_id, "GET", "/sub/", &[]).await;
    assert_eq!(sub.status, 200);
    assert_eq!(sub.body, b"<html>sub</html>");

    // D-A1-11 boundary: no trailing slash, no entry -> not index.html. This
    // service declares no `http_routes` and the request isn't a POST, so it
    // falls all the way through to the JSON-RPC bridge's own 405 -- the
    // meaningful assertion is that the asset layer added no special-casing
    // of its own (never index.html, never a 200).
    let miss = http_request(&conn, &app_service_id, "GET", "/nofallback", &[]).await;
    assert_eq!(
        miss.status, 405,
        "an unknown non-trailing-slash path must not fall back to index.html"
    );
    assert_ne!(miss.body, b"<html>root</html>");

    // Exit criterion 4: a repeat GET with If-None-Match -> 304, empty body.
    let cached =
        http_request(&conn, &app_service_id, "GET", "/", &[("If-None-Match", &etag)]).await;
    assert_eq!(cached.status, 304);
    assert!(cached.body.is_empty(), "a 304 response must carry no body");
    assert_eq!(cached.headers.get("etag").cloned(), Some(etag.clone()));

    // HEAD returns the same headers with no body.
    let head = http_request(&conn, &app_service_id, "HEAD", "/", &[]).await;
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty(), "HEAD must carry no body");
    assert_eq!(head.headers.get("etag").cloned(), Some(etag));

    // Cache-Control by content type: a hashed-looking .js asset is
    // immutable, unlike text/html above.
    let js = http_request(&conn, &app_service_id, "GET", "/assets/app.js", &[]).await;
    assert_eq!(js.status, 200);
    assert_eq!(
        js.headers.get("cache-control").map(String::as_str),
        Some("public, max-age=31536000, immutable")
    );

    let _ = peer.shutdown().await;
    ctx.teardown().await;
}

// ---------------------------------------------------------------------
// Matrix 4: cross-service isolation
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_static_asset_cross_service_isolation() {
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(9103, 9104, 9105).await;
    ctx.substrate_client.inject_kek("22".repeat(32)).await.expect("inject_kek failed");

    let wasm_bytes = std::fs::read(test_constants::greeter_wasm_path()).unwrap();

    let identity_a = Identity::generate().unwrap();
    let service_a = substrate::derive_did_key(&identity_a.public_key());
    let archive_a = make_asset_archive(&[("secret.txt", b"service A's content")]);
    deploy(
        &ctx.substrate_client,
        &service_a,
        wasm_asset_manifest(wasm_bytes.clone(), archive_a, Visibility::Public, None),
    )
    .await;

    let identity_b = Identity::generate().unwrap();
    let service_b = substrate::derive_did_key(&identity_b.public_key());
    let archive_b = make_asset_archive(&[("secret.txt", b"service B's content")]);
    deploy(
        &ctx.substrate_client,
        &service_b,
        wasm_asset_manifest(wasm_bytes, archive_b, Visibility::Public, None),
    )
    .await;

    let mut peer_a = connect_peer(&service_a, &ctx.substrate_mechanisms);
    peer_a.connect().await.expect("peer A failed to connect");
    let conn_a = peer_a.connection().expect("peer A has no live connection");
    let resp_a = http_request(&conn_a, &service_a, "GET", "/secret.txt", &[]).await;
    assert_eq!(resp_a.status, 200);
    assert_eq!(resp_a.body, b"service A's content");

    let mut peer_b = connect_peer(&service_b, &ctx.substrate_mechanisms);
    peer_b.connect().await.expect("peer B failed to connect");
    let conn_b = peer_b.connection().expect("peer B has no live connection");
    let resp_b = http_request(&conn_b, &service_b, "GET", "/secret.txt", &[]).await;
    assert_eq!(resp_b.status, 200);
    assert_eq!(
        resp_b.body, b"service B's content",
        "a connection scoped to service B must never see service A's asset bytes"
    );

    let _ = peer_a.shutdown().await;
    let _ = peer_b.shutdown().await;
    ctx.teardown().await;
}

// ---------------------------------------------------------------------
// Matrix 7: private visibility is byte-identical to no bundle
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_static_asset_private_visibility_matches_no_bundle() {
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(9106, 9107, 9108).await;
    ctx.substrate_client.inject_kek("23".repeat(32)).await.expect("inject_kek failed");

    let wasm_bytes = std::fs::read(test_constants::greeter_wasm_path()).unwrap();

    // D-A1-8: a non-public bundle must be byte-identical, from the outside,
    // to no bundle at all -- proven by deploying one service each way and
    // comparing their responses to the same request, rather than assuming
    // a specific status code (which depends on whatever else the service
    // does or doesn't have declared).
    let private_identity = Identity::generate().unwrap();
    let private_service_id = substrate::derive_did_key(&private_identity.public_key());
    let archive = make_asset_archive(&[("index.html", b"<html>private</html>")]);
    deploy(
        &ctx.substrate_client,
        &private_service_id,
        wasm_asset_manifest(wasm_bytes.clone(), archive, Visibility::Private, None),
    )
    .await;

    let no_assets_identity = Identity::generate().unwrap();
    let no_assets_service_id = substrate::derive_did_key(&no_assets_identity.public_key());
    deploy(&ctx.substrate_client, &no_assets_service_id, wasm_manifest_without_assets(wasm_bytes))
        .await;

    let mut private_peer = connect_peer(&private_service_id, &ctx.substrate_mechanisms);
    private_peer.connect().await.expect("private peer failed to connect");
    let private_conn = private_peer.connection().expect("private peer has no live connection");
    let private_resp = http_request(&private_conn, &private_service_id, "GET", "/", &[]).await;

    let mut bare_peer = connect_peer(&no_assets_service_id, &ctx.substrate_mechanisms);
    bare_peer.connect().await.expect("bare peer failed to connect");
    let bare_conn = bare_peer.connection().expect("bare peer has no live connection");
    let bare_resp = http_request(&bare_conn, &no_assets_service_id, "GET", "/", &[]).await;

    assert_eq!(
        private_resp.status, bare_resp.status,
        "a private bundle must refuse exactly like no bundle at all"
    );
    assert_eq!(private_resp.body, bare_resp.body);
    assert_ne!(private_resp.body, b"<html>private</html>", "the private content must never leak");

    let _ = private_peer.shutdown().await;
    let _ = bare_peer.shutdown().await;
    ctx.teardown().await;
}

// ---------------------------------------------------------------------
// Multi-chunk round trip
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_static_asset_multi_chunk_round_trip() {
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(9109, 9110, 9111).await;
    ctx.substrate_client.inject_kek("24".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let wasm_bytes = std::fs::read(test_constants::greeter_wasm_path()).unwrap();
    // Larger than the router's 64 KiB blob-store read-chunk size, so the
    // streamed response spans several `read-chunk` native-dispatch calls.
    let big: Vec<u8> = (0..300_000usize).map(|i| (i % 251) as u8).collect();
    let archive = make_asset_archive(&[("big.bin", &big)]);
    deploy(
        &ctx.substrate_client,
        &app_service_id,
        wasm_asset_manifest(wasm_bytes, archive, Visibility::Public, None),
    )
    .await;

    let mut peer = connect_peer(&app_service_id, &ctx.substrate_mechanisms);
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");

    let resp = http_request(&conn, &app_service_id, "GET", "/big.bin", &[]).await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body.len(), big.len());
    assert_eq!(resp.body, big, "a multi-chunk asset must round-trip byte-identical");

    let _ = peer.shutdown().await;
    ctx.teardown().await;
}

// ---------------------------------------------------------------------
// Assets and `http_routes` coexist without one shadowing the other
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_static_assets_and_http_routes_coexist() {
    let _ = ring::default_provider().install_default();
    let ctx = SubstrateTestContext::setup(9112, 9113, 9114).await;
    ctx.substrate_client.inject_kek("25".repeat(32)).await.expect("inject_kek failed");

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    let wasm_bytes = std::fs::read(test_constants::greeter_wasm_path()).unwrap();
    let archive = make_asset_archive(&[("index.html", b"<html>hi</html>")]);
    let custom_config = serde_json::json!({
        "http_routes": [
            {"method": "GET", "path": "/api/comments", "target": "data-layer",
             "operation": "query", "collection": "comments"},
        ]
    });
    deploy(
        &ctx.substrate_client,
        &app_service_id,
        wasm_asset_manifest(wasm_bytes, archive, Visibility::Public, Some(custom_config)),
    )
    .await;

    let mut peer = connect_peer(&app_service_id, &ctx.substrate_mechanisms);
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");

    // The asset path still resolves to the bundle.
    let asset_resp = http_request(&conn, &app_service_id, "GET", "/index.html", &[]).await;
    assert_eq!(asset_resp.status, 200);
    assert_eq!(asset_resp.body, b"<html>hi</html>");

    // The declared route still reaches `data-layer`, not the asset table.
    // `comments` was never created, so `data-layer::query` legitimately
    // answers its own "collection not found" (-32011, mapped to 404) --
    // if this path had instead fallen through both the asset layer and the
    // route table (i.e. `http_routes` hadn't actually taken effect), it
    // would land on the JSON-RPC bridge's 405 with a *different* error
    // code (-32603), not this one -- so this status+code pair together
    // are what actually prove the request reached the route bridge.
    let route_resp = http_request(&conn, &app_service_id, "GET", "/api/comments", &[]).await;
    assert_eq!(route_resp.status, 404);
    let body: serde_json::Value = serde_json::from_slice(&route_resp.body)
        .expect("a route-bridge error must be a JSON-RPC error envelope, not an empty asset 404");
    assert_eq!(
        body["error"]["code"],
        serde_json::json!(-32011),
        "must be data-layer's own collection-not-found code, proving the request reached the \
         route bridge: {body}"
    );

    let _ = peer.shutdown().await;
    ctx.teardown().await;
}
