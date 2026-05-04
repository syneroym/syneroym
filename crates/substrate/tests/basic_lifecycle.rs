use assert_cmd::assert::OutputAssertExt;
use assert_cmd::cargo::CommandCargoExt;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;
use syneroym_bindings::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};
use syneroym_core::config::SubstrateConfig;
use tempfile::NamedTempFile;
use tracing::debug;

fn send_ctrl_c(#[allow(unused_variables)] pid: u32) {
    #[cfg(unix)]
    {
        // SAFETY: libc::kill() is called with:
        // - pid: a valid process ID from std::process::Child::id()
        // - signal: libc::SIGINT, a standard signal supported on Unix systems
        // The function is safe when PID is valid (which it is from Child::id())
        // and the signal number is correct. No memory is modified by this call.
        unsafe {
            libc::kill(pid as i32, libc::SIGINT);
        }
    }
    #[cfg(windows)]
    {
        // On Windows, there is no direct equivalent to sending SIGINT to a specific PID.
        // GenerateConsoleCtrlEvent sends a Ctrl+C event to the console group.
        #[link(name = "kernel32")]
        extern "system" {
            fn GenerateConsoleCtrlEvent(dwCtrlEvent: u32, dwProcessGroupId: u32) -> i32;
        }
        // SAFETY: GenerateConsoleCtrlEvent is a stable Windows API that only sends a console event.
        // Parameters: 0 = CTRL_C_EVENT, 0 = broadcast to all processes in current console group.
        // No memory access or modification occurs. Function is thread-safe.
        unsafe {
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
    use syneroym_core::config::{CoordinatorIrohConfig, CoordinatorRole, ServiceRegistryRole};

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
    config.logging.target = syneroym_core::config::LogTarget::Stdout;

    config.roles.coordinator = Some(CoordinatorRole {
        iroh: Some(CoordinatorIrohConfig {
            enable_relay: true,
            http_bind_address: format!("0.0.0.0:{}", IROH_PORT),
            ..Default::default()
        }),
        ..Default::default()
    });
    config.roles.community_registry = Some(ServiceRegistryRole { ..Default::default() });
    config.substrate.registry_url = Some("http://localhost:8080".to_string());
    config.uplink.iroh =
        Some(syneroym_core::config::IrohRelayConfig { relay_url: IROH_RELAY_URL.to_string() });

    // Generate identity beforehand so we know it
    let substrate_identity_state = syneroym_substrate::identity::setup_substrate_identity(
        &config.identity,
        &config.app_data_dir,
    )
    .expect("Failed to setup identity");
    let substrate_service_id = substrate_identity_state.did.clone();

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

    let runtime =
        syneroym_substrate::init(config.clone()).await.expect("Failed to initialize runtime");

    // Spawn the entire substrate in a background task.
    let substrate_handle = tokio::spawn(async move {
        syneroym_substrate::run_with_signal(config, runtime, async {
            let _ = shutdown_rx.recv().await;
        })
        .await
        .expect("Substrate failed to run");
    });

    let mut substrate_client = syneroym_sdk::SyneroymClient::new(
        substrate_service_id.clone(),
        "http://localhost:8080".to_string(),
    );

    substrate_client
        .wait_for_ready(Duration::from_secs(30))
        .await
        .expect("Substrate did not become available in time");

    // Deploy a test WASM application
    debug!(">>> Starting STEP 1: Deploy");
    let wasm_bytes = std::fs::read(
        "../../test-components/greeter/target/wasm32-wasip2/release/syneroym_test_greeter.wasm",
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
        substrate_client.request("orchestrator", "deploy", deploy_params),
    )
    .await
    .expect("deploy request timed out")
    .expect("Deploy request failed");

    assert_eq!(deploy_res.result, serde_json::json!({"status": "deployed"}));
    debug!(">>> Finished STEP 1: Deploy");

    // Interact with the running WASM application
    debug!(">>> Starting STEP 2: Run");
    let substrate_info =
        substrate_client.lookup().await.expect("Failed to lookup substrate info from registry");
    let substrate_mechanisms = substrate_info.info.mechanisms;

    let mut app_client = syneroym_sdk::SyneroymClient::new_with_mechanisms(
        "greeter-service".to_string(),
        substrate_mechanisms,
    );
    app_client.connect().await.expect("Failed to connect to app on substrate");

    let app_res = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        app_client.request(
            "syneroym-test:greeter/greet@0.1.0",
            "greet",
            serde_json::json!(["tester"]),
        ),
    )
    .await
    .expect("app run request timed out")
    .expect("App request failed");

    assert_eq!(
        app_res.result,
        serde_json::json!("Hello, tester! Greetings from greeter::greet::greet")
    );
    debug!(">>> Finished STEP 2: Run");

    // Trigger graceful shutdown.
    let _ = shutdown_tx.send(()).await;

    // Await the substrate handle to ensure it shuts down cleanly.
    let result = substrate_handle.await;
    assert!(result.is_ok(), "Substrate task should shut down cleanly without panicking.");
}
