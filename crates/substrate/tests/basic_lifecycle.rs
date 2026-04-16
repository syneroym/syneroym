use assert_cmd::assert::OutputAssertExt;
use assert_cmd::cargo::CommandCargoExt;
use iroh::{EndpointAddr, RelayMap, RelayMode, RelayUrl};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;
use syneroym_bindings::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};
use syneroym_core::config::SubstrateConfig;
use tempfile::NamedTempFile;
use tracing::{debug, error};

fn send_ctrl_c(#[allow(unused_variables)] pid: u32) {
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid as i32, libc::SIGINT);
        }
    }
    #[cfg(windows)]
    {
        // On Windows, there is no direct equivalent to sending SIGINT to a specific PID.
        #[link(name = "kernel32")]
        extern "system" {
            fn GenerateConsoleCtrlEvent(dwCtrlEvent: u32, dwProcessGroupId: u32) -> i32;
        }
        unsafe {
            // 0 = CTRL_C_EVENT
            GenerateConsoleCtrlEvent(0, 0);
        }
    }
}

#[test]
// The stub tests basic binary execution (version or help)
fn substrate_integration_stub() {
    let mut command = Command::cargo_bin("syneroym-substrate").unwrap();

    // The stub tests basic binary execution (version or help)
    command.arg("--help");
    command.assert().success();
}

// Test basic binary execution and graceful shutdown on Ctrl-C.
// This is a black-box test that runs the binary as a subprocess.
#[tokio::test]
async fn test_run_finishes_on_ctrl_c() {
    // Create a temporary config file to explicitly enable the client_gateway role
    let mut config_file = NamedTempFile::new().expect("Failed to create temp config file");
    let config_toml = r#"
    [roles.client_gateway]
    "#;
    write!(config_file, "{}", config_toml).expect("Failed to write to temp config file");

    let mut command = Command::cargo_bin("syneroym-substrate").unwrap();

    // Spawn the substrate process with piped stdout
    let mut child = command
        .arg("run")
        .arg("--config")
        .arg(config_file.path())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to spawn syneroym-substrate process");

    // Wait for the process to initialize and start running by reading its stdout
    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).expect("Failed to read from child stdout");
        if bytes_read == 0 {
            panic!("Process closed stdout before reaching running state");
        }
        // Look for a reliable log line indicating the the process is running with reasonable confidence. Enough for sanity purpose.
        if line.contains("running client gateway") {
            break;
        }
    }

    // Give the process a brief moment to ensure signal handler registration completes
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send SIGINT (Ctrl-C) to the child process
    send_ctrl_c(child.id());

    // Wait for the process to exit
    let status = child.wait().expect("Failed to wait on child process");

    // Verify the process exited successfully (graceful shutdown)
    assert!(
        status.success(),
        "Process did not exit successfully after SIGINT, status: {:?}",
        status
    );
}

const IROH_PORT: u16 = 3340;
const IROH_RELAY_URL: &str = "http://localhost:3340";

/// This in-process integration test starts a substrate,
/// runs operations done over a typical substrate lifetime, finally shutting it down
#[tokio::test]
#[cfg(feature = "app_sandbox")]
async fn test_in_process_lifecycle_shutdown_on_ctrl_c() {
    use syneroym_core::config::{CoordinatorIrohConfig, CoordinatorRole};

    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let base_path = temp_dir.path();
    // Construct the configuration programmatically.
    let mut config = SubstrateConfig {
        app_local_data_dir: base_path.join("data"),
        app_data_dir: base_path.join("user_data"),
        app_cache_dir: base_path.join("cache"),
        app_log_dir: base_path.join("logs"),

        profile: "full".to_string(),
        ..SubstrateConfig::default()
    };
    config.resolve_paths();

    // Since this test runs the substrate in-process, we can't rely on `cargo test` capturing stdout/stderr.
    // So we configure the substrate to write logs to a temporary file and avoid large outputs while running tests.
    // NOTE: Comment for debugging purpose uncomment otherwise.
    config.logging.target = syneroym_core::config::LogTarget::File;

    config.roles.coordinator = Some(CoordinatorRole {
        iroh: Some(CoordinatorIrohConfig {
            enable_relay: true,
            http_bind_address: format!("0.0.0.0:{}", IROH_PORT),
            ..Default::default()
        }),
        ..Default::default()
    });
    config.uplink.iroh =
        Some(syneroym_core::config::IrohRelayConfig { relay_url: IROH_RELAY_URL.to_string() });

    // Generate identity beforehand so we know it
    let substrate_identity_state = syneroym_substrate::identity::setup_substrate_identity(
        &config.identity,
        &config.app_data_dir,
    )
    .expect("Failed to setup identity");
    let substrate_service_id =
        syneroym_identity::substrate::resolve_did_z32(&substrate_identity_state.did)
            .expect("Failed to resolve did")
            .to_string();

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

    let runtime =
        syneroym_substrate::init(config.clone()).await.expect("Failed to initialize runtime");
    let iroh_endpoint_addr = runtime
        .connection_router
        .endpoint_addr()
        .expect("Runtime did not expose an endpoint address");

    // Spawn the entire substrate in a background task.
    let mut substrate_handle = tokio::spawn(async move {
        syneroym_substrate::run_with_signal(config, runtime, async {
            let _ = shutdown_rx.recv().await;
        })
        .await
        .expect("Substrate failed to run");
    });

    // Wait for the substrate to become fully available by polling its health check endpoint.
    let (endpoint, conn) = abort_if_failed(
        wait_for_substrate(&substrate_service_id, iroh_endpoint_addr.clone()),
        &mut substrate_handle,
        &shutdown_tx,
        "Substrate did not become available in time",
    )
    .await;

    // Deploy a test WASM application
    debug!(">>> Starting STEP 1: Deploy");
    let wasm_bytes = std::fs::read(
        "../../test-components/greeter/target/wasm32-wasip1/debug/syneroym_test_greeter.wasm",
    )
    .expect("Failed to read compiled test WASM component");
    let deploy_params = serde_json::to_value((
        "greeter-service".to_string(),
        vec!["syneroym-test:greeter/greet@0.1.0"],
        DeployManifest {
            config: ServiceConfig { env: vec![], args: vec![], custom_config: None },
            service_type: ServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(wasm_bytes),
                hash: None,
            }),
        },
    ))
    .expect("Failed to serialize deploy params");
    let deploy_res = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        send_native_json_rpc_request(&substrate_service_id, &conn, "deploy", deploy_params),
    )
    .await
    .expect("deploy request timed out");
    assert!(deploy_res.is_some(), "Deploy request failed");
    let deploy_res = deploy_res.unwrap();
    assert_eq!(deploy_res.result, serde_json::json!({"status": "deployed"}));
    debug!(">>> Finished STEP 1: Deploy");

    // Interact with the running WASM application
    debug!(">>> Starting STEP 2: Run");
    let app_res = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        send_json_rpc_request(
            "greeter-service",
            "syneroym-test:greeter/greet@0.1.0",
            &conn,
            "greet",
            serde_json::json!(["tester"]),
        ),
    )
    .await
    .expect("app run request timed out");
    assert!(app_res.is_some(), "App request failed");
    let app_res = app_res.unwrap();
    assert_eq!(
        app_res.result,
        serde_json::json!("Hello, tester! Greetings from greeter::greet::greet")
    );
    debug!(">>> Finished STEP 2: Run");

    // Teardown / Graceful Shutdown ---
    conn.close(0u8.into(), b"done");
    endpoint.close().await;

    // Trigger graceful shutdown.
    let _ = shutdown_tx.send(()).await;

    // Await the substrate handle to ensure it shuts down cleanly.
    let result = substrate_handle.await;
    assert!(result.is_ok(), "Substrate task should shut down cleanly without panicking.");
}

// Helper used by `app_sandbox` feature tests; suppresses warning when feature is disabled.
#[allow(dead_code)]
async fn abort_if_failed<F, T>(
    step: F,
    handle: &mut tokio::task::JoinHandle<()>,
    shutdown_tx: &tokio::sync::mpsc::Sender<()>,
    msg: &str,
) -> T
where
    F: std::future::Future<Output = Option<T>>,
{
    if let Some(res) = step.await {
        res
    } else {
        abort_substrate(handle, shutdown_tx, msg).await;
        unreachable!()
    }
}

// Helper used by `app_sandbox` feature tests; suppresses warning when feature is disabled.
#[allow(dead_code)]
async fn abort_substrate(
    handle: &mut tokio::task::JoinHandle<()>,
    shutdown_tx: &tokio::sync::mpsc::Sender<()>,
    msg: &str,
) {
    let _ = shutdown_tx.send(()).await;
    let _ = handle.await;
    panic!("{}", msg);
}

// Helper used by `app_sandbox` feature tests; suppresses warning when feature is disabled.
#[allow(dead_code)]
async fn wait_for_substrate(
    service_id: &str,
    endpoint_addr: EndpointAddr,
) -> Option<(iroh::Endpoint, iroh::endpoint::Connection)> {
    let ep_bldr = iroh::Endpoint::empty_builder().relay_mode(RelayMode::Custom(RelayMap::from(
        IROH_RELAY_URL.to_string().parse::<RelayUrl>().unwrap(),
    )));

    let endpoint = ep_bldr.bind().await.ok()?;

    tokio::time::timeout(Duration::from_secs(300), async {
        loop {
            if let Ok(Ok(conn)) = tokio::time::timeout(
                Duration::from_millis(1500),
                endpoint.connect(endpoint_addr.clone(), syneroym_router::SYNEROYM_ALPN),
            )
            .await
                && check_substrate_available(service_id, &conn).await
            {
                return Some((endpoint, conn));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .ok()
    .flatten()
}

// Helper used by `app_sandbox` feature tests; suppresses warning when feature is disabled.
#[allow(dead_code)]
async fn check_substrate_available(service_id: &str, conn: &iroh::endpoint::Connection) -> bool {
    if let Some(response) = send_native_json_rpc_request(
        service_id,
        conn,
        "readyz",
        serde_json::Value::Object(serde_json::Map::new()),
    )
    .await
        && response.result == serde_json::json!({"status": "ok"})
    {
        return true;
    }
    false
}

/// A reusable helper for sending JSON-RPC requests over Iroh Streams in E2E tests.
// Helper used by `app_sandbox` feature tests; suppresses warning when feature is disabled.
async fn send_native_json_rpc_request(
    service_id: &str,
    conn: &iroh::endpoint::Connection,
    method: &str,
    params: serde_json::Value,
) -> Option<syneroym_rpc::JsonRpcResponse> {
    send_json_rpc_request(service_id, "orchestrator", conn, method, params).await
}

async fn send_json_rpc_request(
    service_id: &str,
    interface_name: &str,
    conn: &iroh::endpoint::Connection,
    method: &str,
    params: serde_json::Value,
) -> Option<syneroym_rpc::JsonRpcResponse> {
    let (send, recv): (iroh::endpoint::SendStream, iroh::endpoint::RecvStream) =
        match tokio::time::timeout(Duration::from_millis(1500), conn.open_bi()).await {
            Ok(Ok(streams)) => streams,
            Ok(Err(error)) => {
                error!(">>> Failed to open stream for method {method}: {error}");
                return None;
            }
            Err(_) => {
                error!(">>> Timed out opening stream for method {method}");
                return None;
            }
        };

    send_json_rpc_request_over_stream(service_id, interface_name, send, recv, method, params).await
}

async fn send_json_rpc_request_over_stream(
    service_id: &str,
    interface_name: &str,
    mut send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
    method: &str,
    params: serde_json::Value,
) -> Option<syneroym_rpc::JsonRpcResponse> {
    use tokio::io::AsyncBufReadExt;

    let preamble = format!("json-rpc://{}.{}\n", interface_name, service_id);
    if send.write_all(preamble.as_bytes()).await.is_err() {
        return None;
    }

    let request = syneroym_rpc::JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params,
        id: Some(serde_json::Value::Number(1.into())),
    };
    let mut req_bytes = serde_json::to_vec(&request).unwrap();
    req_bytes.push(b'\n');
    if send.write_all(&req_bytes).await.is_err() {
        return None;
    }
    debug!(">>> Wrote request for method: {}", method);
    if send.finish().is_err() {
        return None;
    };

    let mut resp_buf = Vec::new();
    let mut reader = tokio::io::BufReader::new(recv);
    let res =
        if tokio::time::timeout(Duration::from_secs(300), reader.read_until(b'\n', &mut resp_buf))
            .await
            .is_err()
            || resp_buf.is_empty()
        {
            error!(">>> Timed out waiting for response to method: {}", method);
            None
        } else {
            serde_json::from_slice::<syneroym_rpc::JsonRpcResponse>(&resp_buf).ok()
        };
    debug!("got json response for method: {}: {:?}", method, res);
    drop(send);
    res
}
