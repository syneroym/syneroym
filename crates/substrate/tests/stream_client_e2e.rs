#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! M3B Slice 6B end-to-end test (ADR-0014): a real `SyneroymClient` opens a
//! direct raw QUIC stream against a deployed WASM service's registered
//! stream protocol -- proving the initiator doesn't need to be another
//! WASM-hosted service (`SyneroymClient::connection()` is already exposed
//! today; no new client-side plumbing was needed, unlike Slice 6A's
//! `subscribe`). Covers both directions plus the unregistered-protocol
//! failure/security row from task.md's table.

use std::time::Duration;

use rustls::crypto::ring;
use syneroym_core::{
    config::{ClientGatewayRole, IrohParentConfig, LogTarget, SubstrateConfig},
    dht_registry::EndpointMechanism,
    test_constants,
};
use syneroym_identity::{Identity, substrate};
use syneroym_router::{RoutePreamble, RouteProtocol, RouteTransport};
use syneroym_rpc::framing;
use syneroym_sdk::{SyneroymClient, TransportConnection};
use syneroym_substrate::identity;
use tempfile::TempDir;
use tokio::{
    sync::{mpsc, mpsc::Sender},
    task::JoinHandle,
    time,
};

const IROH_PORT: u16 = 7954;
const REGISTRY_PORT: u16 = 7951;
const GATEWAY_PORT: u16 = 7950;
const PROTOCOL: &str = "file-transfer";
const TEST_DRIVER_INTERFACE: &str = test_constants::STREAM_TEST_DRIVER_INTERFACE;

struct SubstrateTestContext {
    #[allow(dead_code)]
    config: SubstrateConfig,
    substrate_client: SyneroymClient,
    registry_url: String,
    substrate_mechanisms: Vec<EndpointMechanism>,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
    temp_dir: TempDir,
}

impl SubstrateTestContext {
    async fn setup(iroh_port: u16, registry_port: u16, gateway_port: u16) -> Self {
        use syneroym_core::config::{CoordinatorIrohConfig, CoordinatorRole, ServiceRegistryRole};

        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let base_path = temp_dir.path();
        let mut config = SubstrateConfig {
            app_local_data_dir: base_path.join("data"),
            app_data_dir: base_path.join("user_data"),
            app_cache_dir: base_path.join("cache"),
            app_log_dir: base_path.join("logs"),
            profile: "full".to_string(),
            ..SubstrateConfig::default()
        };
        config.resolve_paths();
        config.logging.target = LogTarget::Stdout;

        config.roles.coordinator = Some(CoordinatorRole {
            iroh: Some(CoordinatorIrohConfig {
                enable_relay: true,
                http_bind_address: format!("0.0.0.0:{iroh_port}"),
                ..Default::default()
            }),
            ..Default::default()
        });
        config.roles.community_registry = Some(ServiceRegistryRole {
            http_bind_address: format!("0.0.0.0:{registry_port}"),
            ..Default::default()
        });
        let registry_url = format!("http://localhost:{registry_port}");
        config.substrate.registry_url = Some(registry_url.clone());
        config.parent_coordinator.iroh =
            Some(IrohParentConfig { url: format!("http://localhost:{iroh_port}") });
        config.roles.client_gateway = Some(ClientGatewayRole { http_port: gateway_port });

        let substrate_identity_state =
            identity::setup_substrate_identity(&config.identity, &config.app_data_dir)
                .expect("Failed to setup identity");
        let substrate_service_id = substrate_identity_state.did.clone();

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        let runtime =
            syneroym_substrate::init(config.clone()).await.expect("Failed to initialize runtime");

        let config_clone = config.clone();
        let substrate_handle = tokio::spawn(async move {
            syneroym_substrate::run_with_signal(config_clone, runtime, async {
                let _ = shutdown_rx.recv().await;
            })
            .await
            .expect("Substrate failed to run");
        });

        let mut substrate_client =
            SyneroymClient::new(substrate_service_id.clone(), registry_url.clone());
        substrate_client
            .wait_for_ready(Duration::from_secs(30))
            .await
            .expect("Substrate did not become available in time");

        let substrate_info =
            substrate_client.lookup().await.expect("Failed to lookup substrate info from registry");
        let substrate_mechanisms = substrate_info.info.mechanisms;

        Self {
            config,
            substrate_client,
            registry_url,
            substrate_mechanisms,
            shutdown_tx,
            substrate_handle,
            temp_dir,
        }
    }

    async fn teardown(mut self) {
        eprintln!("[teardown] shutting down substrate_client...");
        let _ = self.substrate_client.shutdown().await;
        eprintln!("[teardown] sending shutdown signal...");
        let _ = self.shutdown_tx.send(()).await;
        eprintln!("[teardown] awaiting substrate_handle...");
        let _ = time::timeout(Duration::from_secs(20), self.substrate_handle)
            .await
            .map_err(|_| eprintln!("[teardown] substrate_handle join TIMED OUT"));
        eprintln!("[teardown] done");
    }
}

/// Opens a raw bidirectional QUIC stream against `service_id`'s registered
/// `protocol`, writes the preamble + one framed initial payload, and
/// returns the split (send, recv) halves for the caller to drive the
/// unframed chunk transfer per ADR-0014.
async fn open_stream(
    conn: &TransportConnection,
    service_id: &str,
    protocol: &str,
    dir: &str,
    initial_payload: &[u8],
) -> (iroh::endpoint::SendStream, iroh::endpoint::RecvStream) {
    let TransportConnection::Iroh { conn, .. } = conn;
    eprintln!("[open_stream] calling open_bi...");
    let (mut send, recv) = conn.open_bi().await.expect("open_bi failed");
    eprintln!("[open_stream] open_bi returned");

    let preamble = RoutePreamble {
        transport: RouteTransport::Raw,
        protocol: RouteProtocol::Raw,
        interface: protocol.to_string(),
        service_id: service_id.to_string(),
        enc: None,
        pubkey: None,
        delegation: None,
        dir: dir.parse().ok(),
    };
    eprintln!("[open_stream] writing preamble: {}", preamble.to_preamble_line().trim());
    send.write_all(preamble.to_preamble_line().as_bytes()).await.expect("write preamble failed");
    eprintln!("[open_stream] writing initial frame ({} bytes)", initial_payload.len());
    framing::write_frame(&mut send, initial_payload).await.expect("write initial frame failed");
    eprintln!("[open_stream] initial frame written");

    (send, recv)
}

#[tokio::test]
async fn test_real_client_opens_direct_stream_both_directions() {
    let _ = ring::default_provider().install_default();

    let Ok(wasm_bytes) = std::fs::read(test_constants::stream_test_wasm_path()) else {
        eprintln!(
            "Skipping test_real_client_opens_direct_stream_both_directions: stream-test WASM \
             artifact not found (run `cargo build --target wasm32-wasip2 --release` in \
             test-components/stream-test)"
        );
        return;
    };

    let ctx = SubstrateTestContext::setup(IROH_PORT, REGISTRY_PORT, GATEWAY_PORT).await;

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());

    // The fixture's `init()` touches `data-layer` (creates the `uploads`
    // collection), and this test's substrate instance runs with the
    // default `storage.encryption = true` -- a KEK must be injected before
    // any data-layer-using component can be deployed, or `init()` traps.
    ctx.substrate_client.inject_kek("11".repeat(32)).await.expect("inject_kek failed");

    ctx.substrate_client
        .deploy_svc_wasm(
            app_service_id.clone(),
            vec![TEST_DRIVER_INTERFACE.to_string()],
            wasm_bytes,
            None,
        )
        .await
        .expect("deploy_svc_wasm failed");

    // `register-stream-protocol` runs from the fixture's own `init()`
    // lifecycle hook during deploy; give it a moment to land before the
    // first raw stream request (mirrors messaging_client_e2e.rs's own
    // readiness pattern).
    time::sleep(Duration::from_millis(200)).await;

    eprintln!("[test] connecting peer...");
    let mut peer = SyneroymClient::new_with_mechanisms(
        app_service_id.clone(),
        ctx.substrate_mechanisms.clone(),
    );
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");
    eprintln!("[test] peer connected");

    // --- Download direction ---
    let request_data = b"e2e-download-request";
    eprintln!("[test] opening download stream...");
    let (mut send, mut recv) =
        open_stream(&conn, &app_service_id, PROTOCOL, "download", request_data).await;
    eprintln!("[test] download stream opened, finishing send");
    send.finish().expect("finish send for download request");

    eprintln!("[test] reading download response...");
    let downloaded = time::timeout(Duration::from_secs(10), recv.read_to_end(1024 * 1024))
        .await
        .expect("download timed out")
        .expect("download read failed");
    eprintln!("[test] download complete: {} bytes", downloaded.len());

    let expected_download =
        format!("stream-test:unknown-peer:{}", String::from_utf8_lossy(request_data)).into_bytes();
    assert_eq!(downloaded, expected_download);

    // --- Upload direction ---
    let upload_metadata = b"e2e-upload-metadata";
    let upload_content = b"content pushed over a real QUIC stream end to end".to_vec();
    eprintln!("[test] opening upload stream...");
    let (mut send, _recv) =
        open_stream(&conn, &app_service_id, PROTOCOL, "upload", upload_metadata).await;
    eprintln!("[test] writing upload content...");
    send.write_all(&upload_content).await.expect("upload write failed");
    eprintln!("[test] finishing upload send...");
    send.finish().expect("finish send for upload");
    eprintln!("[test] upload send finished, polling for committed content...");

    let deadline = time::Instant::now() + Duration::from_secs(10);
    let mut stored = String::new();
    let mut attempt = 0u32;
    while time::Instant::now() < deadline {
        attempt += 1;
        eprintln!("[test] get-uploaded-content attempt {attempt}...");
        let response = time::timeout(
            Duration::from_secs(5),
            peer.request(TEST_DRIVER_INTERFACE, "get-uploaded-content", serde_json::json!([])),
        )
        .await
        .expect("get-uploaded-content request timed out")
        .expect("get-uploaded-content request failed");
        eprintln!("[test] get-uploaded-content attempt {attempt} returned: {:?}", response.result);
        stored = response.result.as_str().unwrap_or_default().to_string();
        if !stored.is_empty() {
            break;
        }
        time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(stored, String::from_utf8(upload_content).unwrap());
    eprintln!("[test] assertion passed, shutting down peer...");

    let _ = peer.shutdown().await;
    eprintln!("[test] peer shut down, tearing down context...");
    ctx.teardown().await;
    eprintln!("[test] teardown complete");
}

/// task.md's failure/security test row: "Peer opens a stream against an
/// unregistered protocol namespace -> host rejects the stream cleanly; no
/// panic, no hang." Deploys a plain TCP-typed service (gets the same
/// `EndpointRegistry` wiring as any deployed service, but no
/// `register-stream-protocol` call is ever made against it), then attempts
/// a raw stream against a protocol name nobody registered.
#[tokio::test]
async fn test_unregistered_stream_protocol_rejected_cleanly() {
    let _ = ring::default_provider().install_default();

    let ctx = SubstrateTestContext::setup(IROH_PORT + 1, REGISTRY_PORT + 1, GATEWAY_PORT + 1).await;

    let app_identity = Identity::generate().unwrap();
    let app_service_id = substrate::derive_did_key(&app_identity.public_key());
    ctx.substrate_client
        .deploy_svc_tcp(
            app_service_id.clone(),
            vec![syneroym_sdk::NetworkEndpoint {
                interface_name: "default".to_string(),
                host: "localhost".to_string(),
                port: 30199,
            }],
            None,
        )
        .await
        .expect("deploy_svc_tcp failed");

    let mut peer = SyneroymClient::new_with_mechanisms(
        app_service_id.clone(),
        ctx.substrate_mechanisms.clone(),
    );
    peer.connect().await.expect("peer failed to connect");
    let conn = peer.connection().expect("peer has no live connection");

    let (mut send, mut recv) =
        open_stream(&conn, &app_service_id, "never-registered-protocol", "download", b"req").await;
    send.finish().expect("finish send");

    // The host either closes the stream immediately or errors out of the
    // registry-miss fallback path; either way this must resolve well within
    // a generous timeout, not hang.
    let outcome = time::timeout(Duration::from_secs(10), recv.read_to_end(1024 * 1024)).await;
    assert!(outcome.is_ok(), "unregistered-protocol stream must not hang the peer");
    let buf = outcome.unwrap().unwrap_or_default();
    assert!(buf.is_empty(), "an unregistered protocol must never yield stream bytes");

    let _ = peer.shutdown().await;
    ctx.teardown().await;
}
