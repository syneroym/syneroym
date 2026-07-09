use std::{env, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use clap::Parser;
use iroh::{
    Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl, SecretKey, endpoint::presets::N0,
};
use reqwest::Client;
use serde_json::{Number, Value};
use syneroym_community_registry::EcosystemRegistry;
use syneroym_coordinator_iroh::{CoordinatorIroh, info_endpoint::CoordinatorInfo};
use syneroym_core::{
    config::{
        AccessControl, AppSandboxRole, CoordinatorIrohConfig, CoordinatorRole, RetryPolicy,
        ServiceRegistryRole, SubstrateConfig,
    },
    dht_registry::{EndpointInfo, EndpointMechanism, EndpointType, RegistryClient},
    retry,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::SqliteStorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_router::SYNEROYM_ALPN;
use syneroym_rpc::JsonRpcRequest;
use syneroym_sandbox_wasm::{AppSandboxEngine, WasmResourceQuota};
use tokio::{io::AsyncWriteExt, sync::oneshot, time};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:7964")]
    coordinator_url: String,

    #[arg(long, default_value = "http://127.0.0.1:7961")]
    registry_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    println!("Starting smoke tests...");
    println!("Coordinator URL: {}", args.coordinator_url);
    println!("Registry URL:    {}", args.registry_url);

    let http_client = Client::builder().timeout(Duration::from_secs(5)).build()?;
    let mut info_url = format!("{}/v1/info", args.coordinator_url);

    let is_coordinator_running = http_client.get(&info_url).send().await.is_ok();

    let mut _registry_server = None;
    let mut _coordinator_shutdown = None;

    if !is_coordinator_running {
        println!(
            "No running coordinator detected. Starting temporary in-process coordinator and \
             registry..."
        );

        let mut reg_config = SubstrateConfig::default();
        reg_config.roles.community_registry = Some(ServiceRegistryRole {
            access: AccessControl::String("everyone".to_string()),
            http_bind_address: "127.0.0.1:7961".to_string(),
            parent_registry_url: None,
        });
        let mut registry = EcosystemRegistry::init(&reg_config)
            .await
            .context("Failed to init in-process registry")?;
        registry.spawn().await.context("Failed to spawn in-process registry")?;
        _registry_server = Some(registry);

        let mut coord_config = SubstrateConfig::default();
        coord_config.roles.coordinator = Some(CoordinatorRole {
            access: AccessControl::String("everyone".to_string()),
            tls: None,
            iroh: Some(CoordinatorIrohConfig {
                enable_signalling: false,
                enable_relay: true,
                http_bind_address: "127.0.0.1:7964".to_string(),
                quic_bind_address: "127.0.0.1:7965".to_string(),
                community_registry_url: Some("http://127.0.0.1:7961".to_string()),
                share_in_registry: true,
                idle_timeout_secs: Some(30),
                max_connections: Some(100),
            }),
            webrtc: None,
            transport_bridge: None,
        });

        let coordinator = CoordinatorIroh::init(&coord_config)
            .await
            .context("Failed to init in-process coordinator")?;

        let coord_info_addr =
            coordinator.info_addr().context("Coordinator HTTP address not set")?;
        println!("In-process coordinator listening on info: {}", coord_info_addr);
        info_url = format!("http://{}/v1/info", coord_info_addr);

        let (tx, mut rx) = oneshot::channel::<()>();
        let mut coord_run = coordinator;
        tokio::spawn(async move {
            tokio::select! {
                res = coord_run.run() => {
                    if let Err(e) = res {
                        eprintln!("In-process coordinator run loop error: {:?}", e);
                    }
                }
                _ = &mut rx => {
                    if let Err(e) = coord_run.shutdown().await {
                        eprintln!("In-process coordinator shutdown error: {:?}", e);
                    }
                }
            }
        });

        _coordinator_shutdown = Some(tx);

        time::sleep(Duration::from_millis(1500)).await;
    }

    // Test 1: Connectivity (Coordinator /v1/info)
    println!("\n[Test 1] Connectivity to coordinator...");
    let resp = http_client
        .get(&info_url)
        .send()
        .await
        .context("Failed to connect to coordinator /v1/info")?;

    assert!(
        resp.status().is_success(),
        "Coordinator /v1/info returned error status: {}",
        resp.status()
    );
    let info: CoordinatorInfo =
        resp.json().await.context("Failed to parse CoordinatorInfo JSON")?;
    println!("Coordinator info received successfully!");
    println!("  Substrate ID: {}", info.substrate_id);
    println!("  Status:       {}", info.status);
    println!("  Relay Online: {}", info.relay.as_ref().map(|r| r.online).unwrap_or(false));
    if let Some(conn) = &info.connections {
        println!("  Connections:  active={}/cap={:?}", conn.active, conn.cap);
    }
    if let Some(tls) = &info.tls {
        println!("  TLS Cert Expiry Days: {:?}", tls.cert_expiry_days);
    }

    // Test 2 & 3: Registry & Master Anchor
    println!("\n[Test 2 & 3] Registry registration and master anchor publication...");
    let identity = Identity::generate().context("Failed to generate identity")?;
    let did = derive_did_key(&identity.public_key());
    println!("Generated test identity: {}", did);

    let endpoint_info = EndpointInfo {
        service_id: did.clone(),
        substrate_id: did.clone(),
        endpoint_type: EndpointType::Substrate,
        nickname: Some("smoke-test-node".to_string()),
        mechanisms: vec![EndpointMechanism::Iroh {
            endpoint_addr_bytes: info.endpoint_addr_bytes.clone(),
            relay_url: info.relay_url.clone(),
        }],
        is_private: false,
        ttl: Some(300),
        delegation: None,
    };

    let signed_info = endpoint_info.sign(&identity).context("Failed to sign endpoint info")?;

    let reg_client = RegistryClient::new(true, Some(args.registry_url.clone()));

    println!("Registering endpoint in registry...");
    reg_client.register(&signed_info, true).await.context("Failed to publish to registry")?;
    println!("Endpoint registered successfully!");

    println!("Resolving endpoint from registry...");
    let resolved =
        reg_client.lookup(&did, false).await.context("Failed to resolve from registry")?;
    assert_eq!(resolved.info.service_id, did, "Service ID mismatch");
    println!("Endpoint resolved and verified successfully!");

    // Test 4: Retry mechanism
    println!("\n[Test 4] Inducing transient failure for Iroh QUIC transport retry logic...");
    let retry_policy = RetryPolicy {
        max_attempts: 3,
        initial_backoff_ms: 50,
        backoff_multiplier: 2.0,
        max_backoff_ms: 1000,
    };

    let mut builder = Endpoint::builder(N0);
    if let Some(ref relay_url_str) = info.relay_url
        && let Ok(parsed) = relay_url_str.parse::<RelayUrl>()
    {
        builder = Endpoint::empty_builder().relay_mode(RelayMode::Custom(RelayMap::from(parsed)));
    }
    let test_endpoint = builder.bind().await.context("Failed to bind test endpoint")?;
    let good_addr: EndpointAddr = serde_json::from_slice(&info.endpoint_addr_bytes)?;

    let mut rng = rand::rng();
    let bad_secret_key = SecretKey::generate(&mut rng);
    let bad_node_id = bad_secret_key.public();
    let bad_addr = EndpointAddr::new(bad_node_id);

    let mut attempt = 0;
    let retry_res = retry::retry_with_backoff(&retry_policy, || {
        attempt += 1;
        let test_endpoint = test_endpoint.clone();
        let good_addr = good_addr.clone();
        let bad_addr = bad_addr.clone();
        async move {
            if attempt == 1 {
                println!(
                    "  Attempt 1: Simulating Iroh QUIC transport failure (dialing non-existent \
                     node)"
                );
                let _conn = time::timeout(
                    Duration::from_millis(300),
                    test_endpoint.connect(bad_addr, SYNEROYM_ALPN),
                )
                .await
                .map_err(|_| anyhow::anyhow!("QUIC connection timed out"))??;
                Ok(())
            } else {
                println!("  Attempt {}: Reconnecting to valid Iroh endpoint", attempt);
                match test_endpoint.connect(good_addr, SYNEROYM_ALPN).await {
                    Ok(conn) => match conn.open_bi().await {
                        Ok((mut send, _recv)) => {
                            if let Err(e) = send.write_all(b"PING\n").await {
                                println!("    Write error: {:?}", e);
                                return Err(e.into());
                            }
                            if let Err(e) = send.flush().await {
                                println!("    Flush error: {:?}", e);
                                return Err(e.into());
                            }
                            Ok::<(), anyhow::Error>(())
                        }
                        Err(e) => {
                            println!("    Open bi stream error: {:?}", e);
                            Err(e.into())
                        }
                    },
                    Err(e) => {
                        println!("    Connect error: {:?}", e);
                        Err(e.into())
                    }
                }
            }
        }
    })
    .await;
    assert!(retry_res.is_ok(), "Retry logic should have succeeded on subsequent attempts");
    println!("Iroh QUIC transport retry mechanism verified successfully!");

    // Test 5: Quota trapping
    println!("\n[Test 5] WASM sandbox fuel and memory quota trapping...");
    let wat = r#"
(component
  (core module $m
    (func (export "loop_forever")
      (loop $l
        br $l
      )
    )
    (func (export "allocate_too_much") (param $pages i32) (result i32)
      (memory.grow (local.get $pages))
    )
    (memory (export "memory") 1)
  )
  (core instance $i (instantiate $m))
  (func $loop_forever (canon lift (core func $i "loop_forever")))
  (func $allocate_too_much (param "pages" u32) (result s32) (canon lift (core func $i "allocate_too_much")))
  (instance $interface
    (export "loop-forever" (func $loop_forever))
    (export "allocate-too-much" (func $allocate_too_much))
  )
  (export "test-interface" (instance $interface))
)
"#;
    let mut config = SubstrateConfig::default();
    config.roles.app_sandbox = Some(AppSandboxRole {
        wasm_sandbox: true,
        cpu_limit: 1,
        memory_limit: "64Mi".to_string(),
        max_concurrent_instances: 5,
        default_max_instructions: Some(5_000),
        default_max_memory_bytes: Some(1024 * 1024), // 1MB
    });
    config.storage.blobs_dir = env::temp_dir();

    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(env::temp_dir(), false)?);
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));

    let app_engine =
        AppSandboxEngine::init(&config, vec![], key_store, storage_provider, blob_provider)
            .await
            .context("Failed to init app engine")?;

    let quota = Some(WasmResourceQuota {
        max_instructions: Some(5_000),
        max_memory_bytes: Some(1024 * 1024),
    });

    app_engine
        .compile_and_cache_wasm("smoke_service", wat.as_bytes(), quota)
        .context("Failed to cache WASM component")?;

    let request_loop = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "loop-forever".to_string(),
        params: Value::Array(vec![]),
        id: None,
    };
    let res_loop = app_engine.execute_wasm("smoke_service", "test-interface", &request_loop).await;
    let Err(err) = res_loop else {
        anyhow::bail!("Loop forever should fail");
    };
    let err_msg = err.to_string();
    assert!(err_msg.contains("QuotaExceeded"), "Expected QuotaExceeded error, got: {}", err_msg);
    println!("Fuel quota trapping works! (QuotaExceeded detected)");

    let request_mem = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "allocate-too-much".to_string(),
        params: Value::Array(vec![Value::Number(Number::from(100))]),
        id: None,
    };
    let res_mem = app_engine.execute_wasm("smoke_service", "test-interface", &request_mem).await;
    let Err(err) = res_mem else {
        anyhow::bail!("Memory allocation should fail");
    };
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("MemoryFault") || err_msg.contains("failed to grow memory"),
        "Expected MemoryFault error, got: {}",
        err_msg
    );
    println!("Memory quota trapping works! (MemoryFault/failed to grow detected)");

    if let Some(mut registry) = _registry_server {
        registry.shutdown().await.context("Failed to shutdown registry")?;
    }
    if let Some(tx) = _coordinator_shutdown {
        let _ = tx.send(());
    }

    println!("\nAll smoke tests passed successfully!");
    Ok(())
}
