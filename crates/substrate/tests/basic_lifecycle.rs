use assert_cmd::assert::OutputAssertExt;
use assert_cmd::cargo::CommandCargoExt;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;
use syneroym_bindings::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, TcpManifest, WasmManifest,
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

    let gateway_port = 9090;
    config.roles.client_gateway =
        Some(syneroym_core::config::ClientGatewayRole { http_port: gateway_port });

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
    // Interact with the running WASM application
    debug!(">>> Starting STEP 2: Run");
    let substrate_info =
        substrate_client.lookup().await.expect("Failed to lookup substrate info from registry");
    let substrate_mechanisms = substrate_info.info.mechanisms;

    // Generate a valid DID for the app and deploy it
    let app_identity = syneroym_identity::Identity::generate().unwrap();
    let app_service_id = syneroym_identity::substrate::derive_did_key(&app_identity.public_key());

    let deploy_params = serde_json::to_value((
        app_service_id.clone(),
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

    let mut app_client = syneroym_sdk::SyneroymClient::new_with_mechanisms(
        app_service_id.clone(),
        substrate_mechanisms.clone(),
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

    debug!(">>> Starting STEP 3: Run via HTTP Proxy");
    register_app_in_registry(
        app_service_id.clone(),
        substrate_service_id.clone(),
        substrate_mechanisms.clone(),
        &app_identity,
        "http://localhost:8080",
        "app",
    )
    .await;

    test_http_proxy_invocation(&app_service_id, gateway_port).await;
    debug!(">>> Finished STEP 3: Run via HTTP Proxy");

    // Trigger graceful shutdown.
    let _ = shutdown_tx.send(()).await;

    // Await the substrate handle to ensure it shuts down cleanly.
    let result = substrate_handle.await;
    assert!(result.is_ok(), "Substrate task should shut down cleanly without panicking.");
}

#[tokio::test]
async fn test_tcp_service_lifecycle() {
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
    config.logging.target = syneroym_core::config::LogTarget::Stdout;

    config.roles.coordinator = Some(CoordinatorRole {
        iroh: Some(CoordinatorIrohConfig {
            enable_relay: true,
            http_bind_address: "0.0.0.0:3351".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    });
    config.roles.community_registry = Some(ServiceRegistryRole {
        http_bind_address: "0.0.0.0:8081".to_string(),
        ..Default::default()
    });
    config.substrate.registry_url = Some("http://localhost:8081".to_string());
    config.uplink.iroh = Some(syneroym_core::config::IrohRelayConfig {
        relay_url: "http://localhost:3351".to_string(),
    });

    let gateway_port = 9091;
    config.roles.client_gateway =
        Some(syneroym_core::config::ClientGatewayRole { http_port: gateway_port });

    let substrate_identity_state = syneroym_substrate::identity::setup_substrate_identity(
        &config.identity,
        &config.app_data_dir,
    )
    .expect("Failed to setup identity");
    let substrate_service_id = substrate_identity_state.did.clone();

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
    let runtime =
        syneroym_substrate::init(config.clone()).await.expect("Failed to initialize runtime");

    let substrate_handle = tokio::spawn(async move {
        syneroym_substrate::run_with_signal(config, runtime, async {
            let _ = shutdown_rx.recv().await;
        })
        .await
        .expect("Substrate failed to run");
    });

    let mut substrate_client = syneroym_sdk::SyneroymClient::new(
        substrate_service_id.clone(),
        "http://localhost:8081".to_string(),
    );

    substrate_client
        .wait_for_ready(Duration::from_secs(30))
        .await
        .expect("Substrate did not become available in time");

    // Start miniapp-demo1-web on a random port
    let app_port = 30001;
    let app_addr = std::net::SocketAddr::from(([127, 0, 0, 1], app_port));
    let (app_shutdown_tx, mut app_shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

    let app_data_dir = base_path.join("app_data").to_string_lossy().to_string();
    let app_handle = tokio::spawn(async move {
        let args = miniapp_demo1_web::Args {
            service_name: "tcp-demo-app".to_string(),
            port: app_port,
            data_dir: app_data_dir,
        };
        miniapp_demo1_web::run_server(args, app_addr, async move {
            let _ = app_shutdown_rx.recv().await;
        })
        .await
        .expect("App failed to run");
    });

    // Deploy the app as a "tcp" service
    let app_identity = syneroym_identity::Identity::generate().unwrap();
    let app_service_id = syneroym_identity::substrate::derive_did_key(&app_identity.public_key());

    let deploy_params = serde_json::to_value((
        app_service_id.clone(),
        vec!["default"],
        DeployManifest {
            config: ServiceConfig { env: vec![], args: vec![], custom_config: None },
            service_type: ServiceType::Tcp(TcpManifest {
                host: "127.0.0.1".to_string(),
                port: app_port,
            }),
        },
    ))
    .expect("Failed to serialize deploy params");

    substrate_client
        .request("orchestrator", "deploy", deploy_params)
        .await
        .expect("Deploy request failed");

    // Register in community registry
    let substrate_info = substrate_client.lookup().await.unwrap();
    register_app_in_registry(
        app_service_id.clone(),
        substrate_service_id,
        substrate_info.info.mechanisms,
        &app_identity,
        "http://localhost:8081",
        "tcp-demo-app",
    )
    .await;

    // Test HTTP requests through client_gateway
    let req_client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/", gateway_port);
    let interface_hash = syneroym_core::util::short_hash("default");
    let pubkeyhash = syneroym_core::util::short_hash(&app_service_id);
    let host_header = format!("tcp-demo-app-p{}-i{}.localhost", pubkeyhash, interface_hash);

    // 1. GET /
    let res = req_client.get(&url).header("Host", &host_header).send().await.expect("GET / failed");
    assert!(res.status().is_success());
    let text = res.text().await.unwrap();
    assert!(text.contains("Hello world from tcp-demo-app"));

    // 2. POST /api/comments
    let comment_req = serde_json::json!({"text": "test comment"});
    let res = req_client
        .post(format!("{}api/comments", url))
        .header("Host", &host_header)
        .json(&comment_req)
        .send()
        .await
        .expect("POST /api/comments failed");
    assert_eq!(res.status(), reqwest::StatusCode::CREATED);

    // 3. GET /api/comments
    let res = req_client
        .get(format!("{}api/comments", url))
        .header("Host", &host_header)
        .send()
        .await
        .expect("GET /api/comments failed");
    assert!(res.status().is_success());
    let comments: serde_json::Value = res.json().await.unwrap();
    assert!(!comments.as_array().unwrap().is_empty());
    assert_eq!(comments[0]["text"], "test comment");

    // Shutdown
    let _ = app_shutdown_tx.send(()).await;
    let _ = shutdown_tx.send(()).await;
    let _ = tokio::join!(app_handle, substrate_handle);
}

async fn register_app_in_registry(
    app_service_id: String,
    substrate_service_id: String,
    substrate_mechanisms: Vec<syneroym_core::community_registry::EndpointMechanism>,
    app_identity: &syneroym_identity::Identity,
    registry_url: &str,
    nickname: &str,
) {
    let req_client = reqwest::Client::new();
    let info = syneroym_core::community_registry::EndpointInfo {
        service_id: app_service_id,
        substrate_id: substrate_service_id,
        endpoint_type: syneroym_core::community_registry::EndpointType::Service,
        nickname: Some(nickname.to_string()),
        mechanisms: substrate_mechanisms,
    };
    let info_value = serde_json::to_value(&info).unwrap();
    let canonical_value = syneroym_identity::substrate::canonicalize_json_value(&info_value);
    let canonical_string = serde_json::to_string(&canonical_value).unwrap();
    let signature = app_identity.sign(canonical_string.as_bytes());

    let signed_info = syneroym_core::community_registry::SignedEndpointInfo {
        info,
        signature: z32::encode(&signature.to_bytes()),
    };
    let res = req_client
        .post(format!("{}/register", registry_url))
        .json(&signed_info)
        .send()
        .await
        .expect("Failed to register app_service_id in HTTP registry");
    if !res.status().is_success() {
        let err = res.text().await.unwrap();
        panic!("Registry registration failed: {}", err);
    }
}

async fn test_http_proxy_invocation(app_service_id: &str, gateway_port: u16) {
    let json_req = syneroym_rpc::JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "greet".to_string(),
        params: serde_json::json!(["proxy-tester"]),
        id: Some(serde_json::Value::Number(42.into())),
    };

    let req_client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/", gateway_port);
    let interface_hash = syneroym_core::util::short_hash("syneroym-test:greeter/greet@0.1.0");
    let pubkeyhash = syneroym_core::util::short_hash(app_service_id);
    let host_header = format!("app-p{}-i{}.localhost", pubkeyhash, interface_hash);

    let proxy_res = req_client
        .post(&url)
        .header("Host", host_header)
        .json(&json_req)
        .send()
        .await
        .expect("Failed to send request to client gateway");

    assert!(proxy_res.status().is_success(), "Expected 200 OK, got {}", proxy_res.status());

    let proxy_json: syneroym_rpc::JsonRpcResponse = proxy_res.json().await.unwrap();
    assert_eq!(
        proxy_json.result,
        serde_json::json!("Hello, proxy-tester! Greetings from greeter::greet::greet")
    );
}
