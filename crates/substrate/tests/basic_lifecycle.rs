use assert_cmd::assert::OutputAssertExt;
use assert_cmd::cargo::CommandCargoExt;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;
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
    profile = "enduser"
    [roles.client_gateway]
    http_port = 0
    [roles.observability.health]
    enabled = false
    bind_address = "0.0.0.0:0"
    endpoint = "/health"
    [roles.observability.metrics]
    enabled = false
    bind_address = "0.0.0.0:0"
    endpoint = "/metrics"
    "#;
    write!(config_file, "{}", config_toml).expect("Failed to write to temp config file");

    let mut command = Command::cargo_bin("syneroym-substrate").unwrap();

    // Spawn the substrate process with piped stdout
    let mut child = command
        .arg("run")
        .arg("--config")
        .arg(config_file.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn syneroym-substrate process");

    // Wait for the process to initialize and start running by reading its stdout
    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let stderr = child.stderr.take().expect("Failed to capture stderr");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                let mut err_output = String::new();
                std::io::Read::read_to_string(&mut BufReader::new(stderr), &mut err_output).ok();
                panic!(
                    "Process closed stdout before reaching running state. Stderr:\n{}",
                    err_output
                );
            }
            Ok(_) => {
                if line.contains("running client gateway") {
                    break;
                }
            }
            Err(e) => panic!("Failed to read from child stdout: {}", e),
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

const IROH_PORT: u16 = 7994;
const REGISTRY_PORT: u16 = 7991;
const GATEWAY_PORT: u16 = 7990;
const MOCK_APP_PORT: u16 = 30001;
const MOCK_APP_HTTPS_PORT: u16 = 30002;

/// This in-process integration test context manages the lifecycle of a substrate
/// for testing purposes.
struct SubstrateTestContext {
    #[allow(dead_code)]
    config: SubstrateConfig,
    substrate_client: syneroym_sdk::SyneroymClient,
    substrate_service_id: String,
    gateway_port: u16,
    registry_url: String,
    substrate_mechanisms: Vec<syneroym_core::community_registry::EndpointMechanism>,
    shutdown_tx: tokio::sync::mpsc::Sender<()>,
    substrate_handle: tokio::task::JoinHandle<()>,
    temp_dir: tempfile::TempDir,
}

impl SubstrateTestContext {
    fn gateway_url(&self) -> String {
        format!("http://localhost:{}", self.gateway_port)
    }

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
        config.logging.target = syneroym_core::config::LogTarget::Stdout;

        config.roles.coordinator = Some(CoordinatorRole {
            iroh: Some(CoordinatorIrohConfig {
                enable_relay: true,
                http_bind_address: format!("0.0.0.0:{}", iroh_port),
                ..Default::default()
            }),
            ..Default::default()
        });
        config.roles.community_registry = Some(ServiceRegistryRole {
            http_bind_address: format!("0.0.0.0:{}", registry_port),
            ..Default::default()
        });
        let registry_url = format!("http://localhost:{}", registry_port);
        config.substrate.registry_url = Some(registry_url.clone());
        config.uplink.iroh = Some(syneroym_core::config::IrohRelayConfig {
            relay_url: format!("http://localhost:{}", iroh_port),
        });

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

        let config_clone = config.clone();
        let substrate_handle = tokio::spawn(async move {
            syneroym_substrate::run_with_signal(config_clone, runtime, async {
                let _ = shutdown_rx.recv().await;
            })
            .await
            .expect("Substrate failed to run");
        });

        let mut substrate_client =
            syneroym_sdk::SyneroymClient::new(substrate_service_id.clone(), registry_url.clone());

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
            substrate_service_id,
            gateway_port,
            registry_url,
            substrate_mechanisms,
            shutdown_tx,
            substrate_handle,
            temp_dir,
        }
    }

    async fn teardown(mut self) {
        let _ = self.substrate_client.shutdown().await;
        let _ = self.shutdown_tx.send(()).await;
        let _ = self.substrate_handle.await;
    }
}

#[tokio::test]
#[cfg(feature = "app_sandbox")]
async fn test_substrate_lifecycle_scenarios() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // We use a single substrate instance to run multiple scenarios.
    // Use non-standard ports to avoid conflicts with other tests.
    let ctx = SubstrateTestContext::setup(IROH_PORT, REGISTRY_PORT, GATEWAY_PORT).await;

    // Run WASM app scenario
    test_wasm_app_scenario(&ctx).await;

    // Run TCP service scenario
    test_tcp_service_scenario(&ctx).await;

    ctx.teardown().await;
}

#[cfg(feature = "app_sandbox")]
async fn test_wasm_app_scenario(ctx: &SubstrateTestContext) {
    // Deploy a test WASM application
    debug!(">>> Starting WASM Scenario: Deploy");
    let wasm_bytes = std::fs::read(
        "../../test-components/greeter/target/wasm32-wasip2/release/syneroym_test_greeter.wasm",
    )
    .expect("Failed to read compiled test WASM component");

    // Generate a valid DID for the app and deploy it
    let app_identity = syneroym_identity::Identity::generate().unwrap();
    let app_service_id = syneroym_identity::substrate::derive_did_key(&app_identity.public_key());

    ctx.substrate_client
        .deploy_wasm(
            app_service_id.clone(),
            vec!["syneroym-test:greeter/greet@0.1.0".to_string()],
            wasm_bytes,
        )
        .await
        .expect("SDK Deploy request failed");

    debug!(">>> Finished WASM Scenario: Deploy");

    // Verify listing
    let services = ctx.substrate_client.list_services().await.expect("SDK list_services failed");
    assert!(services.iter().any(|s| s.service_id == app_service_id));
    let svc = services.iter().find(|s| s.service_id == app_service_id).unwrap();
    assert_eq!(svc.endpoint_type, "wasm");
    assert!(svc.interfaces.contains(&"syneroym-test:greeter/greet@0.1.0".to_string()));

    // Interact with the running WASM application via RPC
    debug!(">>> Starting WASM Scenario: Run RPC");
    let mut app_client = syneroym_sdk::SyneroymClient::new_with_mechanisms(
        app_service_id.clone(),
        ctx.substrate_mechanisms.clone(),
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
    app_client.shutdown().await.ok();
    debug!(">>> Finished WASM Scenario: Run RPC");

    // Register in registry and call via HTTP Proxy
    debug!(">>> Starting WASM Scenario: Run via HTTP Proxy");
    register_app_in_registry(
        app_service_id.clone(),
        ctx.substrate_service_id.clone(),
        ctx.substrate_mechanisms.clone(),
        &app_identity,
        &ctx.registry_url,
        "wasm-app",
    )
    .await;

    test_http_proxy_invocation(ctx, &app_service_id, "wasm-app").await;
    debug!(">>> Finished WASM Scenario: Run via HTTP Proxy");
}

async fn test_tcp_service_scenario(ctx: &SubstrateTestContext) {
    debug!(">>> Starting TCP Scenario");

    // Start miniapp-demo1-web on a specific port
    let app_port = MOCK_APP_PORT;
    let app_addr = std::net::SocketAddr::from(([127, 0, 0, 1], app_port));
    let (app_shutdown_tx, mut app_shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

    let app_data_dir = ctx.temp_dir.path().join("app_data_tcp").to_string_lossy().to_string();
    let app_handle = tokio::spawn(async move {
        let args = miniapp_demo1_web::Args {
            service_name: "tcp-demo-app".to_string(),
            port: app_port,
            https_port: MOCK_APP_HTTPS_PORT,
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

    ctx.substrate_client
        .deploy_tcp(
            app_service_id.clone(),
            vec!["default".to_string()],
            "localhost".to_string(),
            app_port,
        )
        .await
        .expect("SDK Deploy TCP request failed");

    // Verify listing
    let services = ctx.substrate_client.list_services().await.expect("SDK list_services failed");
    assert!(services.iter().any(|s| s.service_id == app_service_id));
    let svc = services.iter().find(|s| s.service_id == app_service_id).unwrap();
    assert_eq!(svc.endpoint_type, "tcp");

    // Register in community registry
    register_app_in_registry(
        app_service_id.clone(),
        ctx.substrate_service_id.clone(),
        ctx.substrate_mechanisms.clone(),
        &app_identity,
        &ctx.registry_url,
        "tcp-demo-app",
    )
    .await;

    // Test HTTP requests through client_gateway
    let req_client = reqwest::Client::new();
    let url = format!("{}/", ctx.gateway_url());
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

    // 4. WebSocket Echo Test
    debug!(">>> TCP Scenario: WebSocket Echo Test");
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};

    let ws_url = format!("ws://localhost:{}/ws", ctx.gateway_port);
    let mut request = ws_url.into_client_request().unwrap();
    request.headers_mut().insert("Host", host_header.parse().unwrap());

    let (mut ws_stream, _) = connect_async(request).await.expect("Failed to connect to websocket");

    let test_msg = "hello from websocket";
    ws_stream
        .send(tokio_tungstenite::tungstenite::Message::Text(test_msg.into()))
        .await
        .expect("Failed to send websocket message");

    let response =
        ws_stream.next().await.expect("No response from websocket").expect("Websocket error");

    if let tokio_tungstenite::tungstenite::Message::Text(text) = response {
        let ws_resp: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(ws_resp["recdMsg"].as_str().unwrap().contains(test_msg));
    } else {
        panic!("Expected text message, got {:?}", response);
    }

    // 5. WebSocket Broadcast Test (comment update)
    debug!(">>> TCP Scenario: WebSocket Broadcast Test");
    let comment_req = serde_json::json!({"text": "broadcast test comment"});
    req_client
        .post(format!("{}api/comments", url))
        .header("Host", &host_header)
        .json(&comment_req)
        .send()
        .await
        .expect("POST /api/comments failed for broadcast test");

    let response = ws_stream
        .next()
        .await
        .expect("No broadcast message from websocket")
        .expect("Websocket error");

    if let tokio_tungstenite::tungstenite::Message::Text(text) = response {
        let ws_resp: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(ws_resp["commentUpdateTimestamp"].is_string());
    } else {
        panic!("Expected text message, got {:?}", response);
    }

    // 6. HTTPS Test
    debug!(">>> TCP Scenario: HTTPS Test");
    let https_port = MOCK_APP_HTTPS_PORT;
    let https_app_identity = syneroym_identity::Identity::generate().unwrap();
    let https_app_service_id =
        syneroym_identity::substrate::derive_did_key(&https_app_identity.public_key());

    ctx.substrate_client
        .deploy_tcp(
            https_app_service_id.clone(),
            vec!["default".to_string()],
            "localhost".to_string(),
            https_port,
        )
        .await
        .expect("SDK Deploy TCP (HTTPS) request failed");

    register_app_in_registry(
        https_app_service_id.clone(),
        ctx.substrate_service_id.clone(),
        ctx.substrate_mechanisms.clone(),
        &https_app_identity,
        &ctx.registry_url,
        "tcp-https-app",
    )
    .await;

    // Use a client that accepts invalid certs (since we use a self-signed cert)
    let https_req_client =
        reqwest::Client::builder().danger_accept_invalid_certs(true).build().unwrap();

    // Note: The gateway currently only supports plain HTTP/WS proxying via Host header.
    // For now, we test the HTTPS endpoint directly to ensure it works.
    let app_https_url = format!("https://localhost:{}", https_port);
    let res = https_req_client.get(&app_https_url).send().await.expect("HTTPS GET / failed");

    assert!(res.status().is_success());
    let text = res.text().await.unwrap();
    assert!(text.contains("Hello world from tcp-demo-app"));

    // Shutdown app
    let _ = app_shutdown_tx.send(()).await;
    let _ = app_handle.await;
    debug!(">>> Finished TCP Scenario");
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

async fn test_http_proxy_invocation(
    ctx: &SubstrateTestContext,
    app_service_id: &str,
    nickname: &str,
) {
    let json_req = syneroym_rpc::JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "greet".to_string(),
        params: serde_json::json!(["proxy-tester"]),
        id: Some(serde_json::Value::Number(42.into())),
    };

    let req_client = reqwest::Client::new();
    let url = format!("{}/", ctx.gateway_url());
    let interface_hash = syneroym_core::util::short_hash("syneroym-test:greeter/greet@0.1.0");
    let pubkeyhash = syneroym_core::util::short_hash(app_service_id);
    let host_header = format!("{}-p{}-i{}.localhost", nickname, pubkeyhash, interface_hash);

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
