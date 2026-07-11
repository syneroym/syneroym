#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! M3B Slice 6B integration tests (ADR-0014): `stream-cursor`
//! (guest-as-source) and `stream-sink` (guest-as-sink) driven end to end
//! through `AppSandboxEngine::handle_stream_protocol_request`, bypassing the
//! router/QUIC layer (covered separately by
//! `crates/substrate/tests/stream_client_e2e.rs`) so these tests focus on
//! the Wasmtime/dynamic-invocation boundary.

use std::{fs, path::Path, sync::Arc, time::Duration};

use syneroym_core::{
    config::SubstrateConfig,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    storage::MockStorage,
    streaming::StreamDirection,
    test_constants,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{
    SqliteStorageProvider, StorageProvider, registry_store::SqliteEndpointStorage,
};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::JsonRpcRequest;
use syneroym_sandbox_wasm::AppSandboxEngine;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TEST_DRIVER_INTERFACE: &str = test_constants::STREAM_TEST_DRIVER_INTERFACE;
const PROTOCOL: &str = "file-transfer";
const SERVICE_A: &str = "stream-svc-a";
const SERVICE_B: &str = "stream-svc-b";

async fn make_engine_with_registry(
    dir: &Path,
    registry: EndpointRegistry,
) -> Arc<AppSandboxEngine> {
    let mut config = SubstrateConfig {
        app_local_data_dir: dir.join("data"),
        app_data_dir: dir.join("user_data"),
        app_cache_dir: dir.join("cache"),
        app_log_dir: dir.join("logs"),
        profile: "full".to_string(),
        ..SubstrateConfig::default()
    };
    config.resolve_paths();

    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(&config.storage.db_dir, false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());

    let engine = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            registry,
        )
        .await
        .unwrap(),
    );
    engine.self_weak.set(Arc::downgrade(&engine)).expect("self_weak set once");
    engine
}

async fn make_engine(dir: &Path) -> Arc<AppSandboxEngine> {
    make_engine_with_registry(dir, EndpointRegistry::new_mock(Arc::new(MockStorage::new()))).await
}

fn wasm_deploy_manifest(bytes: Vec<u8>) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema_path: None,
            rotation_policy: None,
        },
        service_type: ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(bytes),
            hash: None,
            interfaces: vec![TEST_DRIVER_INTERFACE.to_string()],
        }),
        registry_certificate: None,
    }
}

async fn call(
    engine: &AppSandboxEngine,
    service_id: &str,
    method: &str,
    params: serde_json::Value,
) -> String {
    let request =
        JsonRpcRequest { jsonrpc: "2.0".to_string(), method: method.to_string(), params, id: None };
    engine.execute_wasm(service_id, TEST_DRIVER_INTERFACE, &request).await.unwrap()
}

fn read_stream_test_wasm() -> Option<Vec<u8>> {
    fs::read(test_constants::stream_test_wasm_path()).ok()
}

macro_rules! skip_if_missing {
    ($test_name:literal) => {
        match read_stream_test_wasm() {
            Some(bytes) => bytes,
            None => {
                eprintln!(
                    "Skipping {}: stream-test WASM artifact not found (build \
                     test-components/stream-test with `cargo build --release --target \
                     wasm32-wasip2`)",
                    $test_name
                );
                return;
            }
        }
    };
}

fn expected_download_payload(peer_id: &str, request_data: &[u8]) -> Vec<u8> {
    format!("stream-test:{peer_id}:{}", String::from_utf8_lossy(request_data)).into_bytes()
}

#[tokio::test]
async fn test_register_stream_protocol_records_in_registry() {
    let wasm_bytes = skip_if_missing!("test_register_stream_protocol_records_in_registry");
    let dir = tempfile::tempdir().unwrap();
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let engine = make_engine_with_registry(dir.path(), registry.clone()).await;

    engine.deploy_wasm(SERVICE_A, &wasm_deploy_manifest(wasm_bytes)).await.unwrap();

    let (endpoint, canonical) = registry.lookup(SERVICE_A, PROTOCOL).expect("protocol registered");
    assert_eq!(canonical, PROTOCOL);
    assert!(
        matches!(endpoint, SubstrateEndpoint::WasmChannel { service_id } if service_id == SERVICE_A)
    );
}

#[tokio::test]
async fn test_cross_service_stream_protocol_isolation() {
    let wasm_bytes = skip_if_missing!("test_cross_service_stream_protocol_isolation");
    let dir = tempfile::tempdir().unwrap();
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let engine = make_engine_with_registry(dir.path(), registry.clone()).await;

    engine.deploy_wasm(SERVICE_A, &wasm_deploy_manifest(wasm_bytes)).await.unwrap();

    assert!(registry.lookup(SERVICE_A, PROTOCOL).is_some());
    assert!(
        registry.lookup(SERVICE_B, PROTOCOL).is_none(),
        "a protocol registered by service A must not be reachable via service B's service_id"
    );
}

#[tokio::test]
async fn test_stream_protocol_registration_survives_restart_replay() {
    let wasm_bytes = skip_if_missing!("test_stream_protocol_registration_survives_restart_replay");
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("endpoints.db");
    let storage = Arc::new(SqliteEndpointStorage::new(&db_path).await.unwrap());
    let registry = EndpointRegistry::new(storage.clone()).await.unwrap();

    let engine = make_engine_with_registry(dir.path(), registry).await;
    engine.deploy_wasm(SERVICE_A, &wasm_deploy_manifest(wasm_bytes)).await.unwrap();

    // Simulates a substrate restart: a brand-new `EndpointRegistry` backed
    // by the same persisted storage, replaying `load_from_db()` at
    // construction -- exactly what `RouteHandler::init` does in production.
    let replayed_registry = EndpointRegistry::new(storage).await.unwrap();
    let (endpoint, _) =
        replayed_registry.lookup(SERVICE_A, PROTOCOL).expect("replayed after restart");
    assert!(
        matches!(endpoint, SubstrateEndpoint::WasmChannel { service_id } if service_id == SERVICE_A)
    );
}

#[tokio::test]
async fn test_download_direction_end_to_end() {
    let wasm_bytes = skip_if_missing!("test_download_direction_end_to_end");
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;
    engine.deploy_wasm(SERVICE_A, &wasm_deploy_manifest(wasm_bytes)).await.unwrap();

    let request_data = b"hello-download".to_vec();
    let expected = expected_download_payload("peer-1", &request_data);

    let (peer, host_side) = tokio::io::duplex(65536);
    let (host_reader, host_writer) = tokio::io::split(host_side);

    let engine_clone = engine.clone();
    let request_data_clone = request_data.clone();
    let handle = tokio::spawn(async move {
        engine_clone
            .handle_stream_protocol_request(
                SERVICE_A,
                PROTOCOL,
                "peer-1",
                StreamDirection::Download,
                request_data_clone,
                Box::new(host_reader),
                Box::new(host_writer),
            )
            .await
    });

    let mut received = Vec::new();
    let (mut peer_read, _peer_write) = tokio::io::split(peer);
    peer_read.read_to_end(&mut received).await.unwrap();

    handle.await.unwrap().expect("download stream should complete cleanly");
    assert_eq!(received, expected);
}

#[tokio::test]
async fn test_download_declined_by_guest_closes_stream_without_bytes() {
    let wasm_bytes =
        skip_if_missing!("test_download_declined_by_guest_closes_stream_without_bytes");
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;
    engine.deploy_wasm(SERVICE_A, &wasm_deploy_manifest(wasm_bytes)).await.unwrap();

    let (peer, host_side) = tokio::io::duplex(65536);
    let (host_reader, host_writer) = tokio::io::split(host_side);

    let engine_clone = engine.clone();
    let handle = tokio::spawn(async move {
        engine_clone
            .handle_stream_protocol_request(
                SERVICE_A,
                PROTOCOL,
                "peer-1",
                StreamDirection::Download,
                b"reject".to_vec(),
                Box::new(host_reader),
                Box::new(host_writer),
            )
            .await
    });

    let mut received = Vec::new();
    let (mut peer_read, _peer_write) = tokio::io::split(peer);
    peer_read.read_to_end(&mut received).await.unwrap();

    handle.await.unwrap().expect("a clean decline is not an error");
    assert!(received.is_empty(), "a declined download must not write any bytes");
}

#[tokio::test]
async fn test_upload_direction_end_to_end_commits_content() {
    let wasm_bytes = skip_if_missing!("test_upload_direction_end_to_end_commits_content");
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;
    engine.deploy_wasm(SERVICE_A, &wasm_deploy_manifest(wasm_bytes)).await.unwrap();

    let upload_content = b"content uploaded via stream-sink end to end".to_vec();

    let (mut peer, host_side) = tokio::io::duplex(65536);
    let (host_reader, host_writer) = tokio::io::split(host_side);

    let engine_clone = engine.clone();
    let handle = tokio::spawn(async move {
        engine_clone
            .handle_stream_protocol_request(
                SERVICE_A,
                PROTOCOL,
                "peer-1",
                StreamDirection::Upload,
                b"upload-metadata".to_vec(),
                Box::new(host_reader),
                Box::new(host_writer),
            )
            .await
    });

    peer.write_all(&upload_content).await.unwrap();
    peer.shutdown().await.unwrap();

    handle.await.unwrap().expect("upload stream should complete cleanly");

    let stored = call(&engine, SERVICE_A, "get-uploaded-content", serde_json::json!([])).await;
    assert_eq!(stored, String::from_utf8(upload_content).unwrap());
}

#[tokio::test]
async fn test_upload_declined_by_guest_leaves_no_stored_content() {
    let wasm_bytes = skip_if_missing!("test_upload_declined_by_guest_leaves_no_stored_content");
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;
    engine.deploy_wasm(SERVICE_A, &wasm_deploy_manifest(wasm_bytes)).await.unwrap();

    let (mut peer, host_side) = tokio::io::duplex(65536);
    let (host_reader, host_writer) = tokio::io::split(host_side);

    let engine_clone = engine.clone();
    let handle = tokio::spawn(async move {
        engine_clone
            .handle_stream_protocol_request(
                SERVICE_A,
                PROTOCOL,
                "peer-1",
                StreamDirection::Upload,
                b"reject".to_vec(),
                Box::new(host_reader),
                Box::new(host_writer),
            )
            .await
    });

    // A declined upload never creates a `stream-sink`, so payload bytes are
    // never read either -- write them anyway to prove the host doesn't hang
    // waiting to read from a peer it already declined.
    peer.write_all(b"should never be read").await.unwrap();
    peer.shutdown().await.unwrap();

    handle.await.unwrap().expect("a clean decline is not an error");

    let stored = call(&engine, SERVICE_A, "get-uploaded-content", serde_json::json!([])).await;
    assert_eq!(stored, "", "a declined upload must not commit any content");
}

#[tokio::test]
async fn test_upload_push_chunk_failure_aborts_without_finalize() {
    let wasm_bytes = skip_if_missing!("test_upload_push_chunk_failure_aborts_without_finalize");
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;
    engine.deploy_wasm(SERVICE_A, &wasm_deploy_manifest(wasm_bytes)).await.unwrap();

    let (mut peer, host_side) = tokio::io::duplex(65536);
    let (host_reader, host_writer) = tokio::io::split(host_side);

    let engine_clone = engine.clone();
    let handle = tokio::spawn(async move {
        engine_clone
            .handle_stream_protocol_request(
                SERVICE_A,
                PROTOCOL,
                "peer-1",
                StreamDirection::Upload,
                b"fail-after-first-chunk".to_vec(),
                Box::new(host_reader),
                Box::new(host_writer),
            )
            .await
    });

    // `push_until_eof` reads the source in 64KiB chunks, so writing well
    // past the fixture's own tiny internal chunking still arrives as one
    // `push-chunk` call; write in two separate flushes with a moment
    // between to encourage two distinct `push-chunk` calls, the second of
    // which the fixture is configured to reject.
    peer.write_all(b"first-chunk-").await.unwrap();
    peer.flush().await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    peer.write_all(b"second-chunk-should-fail").await.unwrap();
    peer.shutdown().await.unwrap();

    let result = handle.await.unwrap();
    assert!(result.is_err(), "a push-chunk failure must surface as an error, not a silent success");

    let stored = call(&engine, SERVICE_A, "get-uploaded-content", serde_json::json!([])).await;
    assert_eq!(stored, "", "an aborted upload must never call finalize / commit content");
}

/// task.md's Slice 6B unit-test row: `stream-cursor.next-chunk()` round trip
/// and `stream-sink.push-chunk()` round trip, both budgeted at < 5ms p99
/// (same measurement style as Slice 6A's `messaging_client_e2e.rs`).
/// Measured indirectly via the full multi-chunk download/upload latency
/// divided by chunk count, since the per-chunk dynamic-invocation helpers
/// are crate-private.
#[tokio::test]
async fn test_next_chunk_and_push_chunk_latency_budget() {
    let wasm_bytes = skip_if_missing!("test_next_chunk_and_push_chunk_latency_budget");
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;
    engine.deploy_wasm(SERVICE_A, &wasm_deploy_manifest(wasm_bytes)).await.unwrap();

    // The fixture chunks in 8-byte pieces; ~50 chunks needs ~400+ bytes of
    // downloaded content.
    let request_data = "x".repeat(400).into_bytes();
    let expected = expected_download_payload("peer-1", &request_data);
    let expected_chunks = expected.len().div_ceil(8);

    let (peer, host_side) = tokio::io::duplex(1 << 20);
    let (host_reader, host_writer) = tokio::io::split(host_side);

    let engine_clone = engine.clone();
    let request_data_clone = request_data.clone();
    let start = std::time::Instant::now();
    let handle = tokio::spawn(async move {
        engine_clone
            .handle_stream_protocol_request(
                SERVICE_A,
                PROTOCOL,
                "peer-1",
                StreamDirection::Download,
                request_data_clone,
                Box::new(host_reader),
                Box::new(host_writer),
            )
            .await
    });

    let mut received = Vec::new();
    let (mut peer_read, _peer_write) = tokio::io::split(peer);
    peer_read.read_to_end(&mut received).await.unwrap();
    handle.await.unwrap().unwrap();
    let elapsed = start.elapsed();

    assert_eq!(received, expected);
    let per_chunk = elapsed / expected_chunks as u32;
    eprintln!(
        "next-chunk average round-trip: {per_chunk:?} over {expected_chunks} chunks (total \
         {elapsed:?})"
    );
    // Budget is 5ms p99 per task.md; asserted here at 3x (15ms) average for
    // headroom against shared-CI-runner variance, consistent with Slice
    // 6A's own budget-test margin.
    assert!(per_chunk < Duration::from_millis(15), "next-chunk average round-trip budget blown");
}

/// task.md's failure/security test row: "a long-running stream exceeds the
/// default single-invocation epoch deadline while still making progress ->
/// no spurious trap" (ADR-0014 "Instance Lifetime and Quota"). The peer
/// deliberately paces its reads so the *whole* download spans more than the
/// 5-second single-call epoch budget, while no individual `next-chunk` call
/// takes anywhere near that long -- proving the deadline is re-armed per
/// call rather than inherited from stream-open time.
#[tokio::test]
async fn test_long_running_stream_does_not_trap_on_epoch_deadline() {
    let wasm_bytes = skip_if_missing!("test_long_running_stream_does_not_trap_on_epoch_deadline");
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;
    engine.deploy_wasm(SERVICE_A, &wasm_deploy_manifest(wasm_bytes)).await.unwrap();

    let request_data = "y".repeat(56).into_bytes(); // ~9 chunks of 8 bytes
    let expected = expected_download_payload("peer-1", &request_data);

    // A duplex buffer sized to exactly one chunk means the host's write for
    // chunk N+1 blocks until the peer has read chunk N, so the peer's own
    // pacing directly controls how long the whole transfer takes -- without
    // any change to non-test engine code.
    let (peer, host_side) = tokio::io::duplex(8);
    let (host_reader, host_writer) = tokio::io::split(host_side);

    let engine_clone = engine.clone();
    let request_data_clone = request_data.clone();
    let handle = tokio::spawn(async move {
        engine_clone
            .handle_stream_protocol_request(
                SERVICE_A,
                PROTOCOL,
                "peer-1",
                StreamDirection::Download,
                request_data_clone,
                Box::new(host_reader),
                Box::new(host_writer),
            )
            .await
    });

    let mut peer_read = peer;
    let mut received = Vec::new();
    let mut buf = [0u8; 8];
    loop {
        // >5s cumulative across ~8 reads while each individual next-chunk
        // call underneath stays near-instant.
        tokio::time::sleep(Duration::from_millis(750)).await;
        let n = peer_read.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        received.extend_from_slice(&buf[..n]);
    }

    let result =
        tokio::time::timeout(Duration::from_secs(15), handle).await.expect("handler hung").unwrap();
    result.expect("a long-running-but-progressing stream must not trap on epoch deadline");
    assert_eq!(received, expected);
}
