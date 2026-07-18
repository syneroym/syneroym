//! Integration tests for the multi-hop relay functionality
//!
//! Simulates the scenario described in the multi-hop-relay-scenario.md
//! which organizes relays, registries, substrates across network boundaries in
//! a hierarchy and tests bidirectional e2e connectivity between clients and
//! substrates across networks.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use iroh::{Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl, SecretKey, protocol::Router};
use reqwest::Client;
use syneroym_community_registry::EcosystemRegistry;
use syneroym_control_plane::ControlPlaneService;
use syneroym_coordinator_iroh::{CoordinatorInfo, CoordinatorIroh};
use syneroym_core::{
    config::{
        AccessControl, CoordinatorIrohConfig, CoordinatorRole, IrohParentConfig,
        ServiceRegistryRole, SubstrateConfig,
    },
    dht_registry::{
        EndpointInfo, EndpointMechanism, EndpointType, RegistryClient, SignedEndpointInfo,
    },
    http_routes::HttpRouteRegistry,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    test_constants,
};
use syneroym_data_blob::ObjectStoreBlobProvider;
use syneroym_data_db::{SqliteStorageProvider, registry_store, traits::StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_identity::{DelegationCertificate, Identity, substrate::derive_did_key};
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_router::{
    AdaptationStage, EncryptionStage, RouteHandler, RouteHandlerDeps, RoutePipeline, RoutePreamble,
    RouteProtocol, RouteTransport, SYNEROYM_ALPN, ServiceStage, TransportStage,
};
use syneroym_rpc::{
    AuthLevel, CallOrigin, CallerContext, CallerProof, NativeDispatchRegistry, NativeInvocation,
    NativeResponse, NativeService, ProxyProtocol, ProxyRequest, RpcResult, SessionContext,
};
use syneroym_sandbox_podman::ContainerEngine;
use syneroym_sandbox_wasm::AppSandboxEngine;
use syneroym_sdk::SyneroymClient;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};
use tokio::time;

/// Mirrors `syneroym_substrate::runtime::build_route_handler_deps`: this
/// test simulates a full substrate node via `RouteHandler::init` directly
/// (bypassing `syneroym-substrate`, whose `runtime` module -- and
/// `build_route_handler_deps` itself -- is private, and which is the
/// top-level composition-root binary crate that already depends on
/// `coordinator_iroh` transitively via `syneroym-coordinator`; depending on
/// it back from here, even as a dev-dependency, would invert that
/// layering), so it must build the same dependency bundle substrate's
/// composition root builds in production. Simplified to the local blob
/// backend only -- these tests never configure S3.
async fn build_test_route_handler_deps(
    config: &SubstrateConfig,
    service_id: &str,
    registry: &EndpointRegistry,
) -> Result<RouteHandlerDeps> {
    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(&config.storage.db_dir, config.storage.encryption)?);
    let bs = &config.storage.blob_store;
    let blob_provider = Arc::new(ObjectStoreBlobProvider::new_local(
        bs.local_root.clone(),
        bs.max_blob_bytes,
        bs.max_service_total_bytes,
    )?);

    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig {
        channel_capacity: config.mqtt.channel_capacity as usize,
    })?);

    let app_sandbox_engine = Arc::new(
        AppSandboxEngine::init(
            config,
            registry.get_all_endpoints(),
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            registry.clone(),
        )
        .await?,
    );
    app_sandbox_engine
        .self_weak
        .set(Arc::downgrade(&app_sandbox_engine))
        .map_err(|_| anyhow::anyhow!("AppSandboxEngine::self_weak set more than once"))?;

    // Mirrors `replay_persisted_subscriptions` in
    // `syneroym_substrate::runtime` (ADR-0010 Finding A1) -- omitting this
    // would silently break any future test in this file that simulates a
    // restart to check a guest's persisted MQTT subscription survives it.
    for (subscribed_service_id, topic) in
        storage_provider.list_all_messaging_subscriptions().await?
    {
        if let Err(e) =
            app_sandbox_engine.register_internal_subscription(&subscribed_service_id, &topic).await
        {
            tracing::warn!(
                service_id = %subscribed_service_id,
                topic = %topic,
                error = %e,
                "Failed to replay messaging subscription on startup"
            );
        }
    }

    let podman_path = config
        .roles
        .podman_sandbox
        .as_ref()
        .map(|cfg| cfg.podman_path.clone())
        .unwrap_or_else(|| "podman".to_string());
    let podman_sandbox_engine = Arc::new(ContainerEngine::new(
        podman_path,
        &config.app_local_data_dir,
        Some(storage_provider.clone()),
    ));

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());

    let control_plane_service = ControlPlaneService::init(
        service_id.to_string(),
        service_id.to_string(),
        app_sandbox_engine.clone(),
        podman_sandbox_engine,
        registry.clone(),
        config.hosted_apps_dir(),
        key_store.clone(),
        storage_provider.clone(),
        blob_provider,
        messaging_broker.clone(),
        native_dispatch.clone(),
        http_routes.clone(),
    )
    .await?;

    Ok(RouteHandlerDeps {
        key_store,
        storage_provider,
        app_sandbox_engine,
        messaging_broker,
        native_dispatch,
        http_routes,
        control_plane_service: Arc::new(control_plane_service),
    })
}

/// Minimal `DeployManifest` for `AppSandboxEngine::deploy_wasm` (M04A Slice
/// A1's cross-node proxy test) -- no `custom_config`/quota, matching
/// `lifecycle_hooks.rs`'s own `wasm_deploy_manifest` helper.
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
            interfaces: vec![],
        }),
        registry_certificate: None,
    }
}

/// Polls `ep.addr()` until it carries at least one direct address (M04A
/// Slice A1's cross-node proxy test, direct-connect-only, no relay).
/// `Endpoint::online()` is unsuitable here -- it waits for *both* a relay
/// connection *and* a local address, and these test endpoints configure no
/// relay at all, so `online()` would never resolve.
async fn wait_for_local_addr(ep: &Endpoint) -> EndpointAddr {
    for _ in 0..50 {
        let addr = ep.addr();
        if !addr.is_empty() {
            return addr;
        }
        time::sleep(Duration::from_millis(20)).await;
    }
    ep.addr()
}

/// Like `create_signed_info`, but preserves the endpoint's real direct
/// addresses instead of pruning them to a bare `EndpointId` (M04A Slice A1's
/// cross-node proxy test needs this: `create_signed_info`'s pruning is fine
/// for `test_inbound_relay`/`test_outbound_relay` above, whose peers
/// reconnect via a *relay* URL alongside the pruned id, but this test's
/// endpoints have no relay at all -- direct addresses are the only
/// addressing information available).
fn create_signed_info_with_full_addr(
    identity: &Identity,
    service_id: &str,
    endpoint_addr: &EndpointAddr,
) -> SignedEndpointInfo {
    let endpoint_addr_bytes = serde_json::to_vec(endpoint_addr).unwrap();
    let info = EndpointInfo {
        service_id: service_id.to_string(),
        substrate_id: service_id.to_string(),
        endpoint_type: EndpointType::Substrate,
        nickname: Some("test-node".to_string()),
        mechanisms: vec![EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url: None }],
        is_private: false,
        ttl: None,
        delegation: None,
    };
    info.sign(identity).unwrap()
}

fn create_signed_info(
    identity: &Identity,
    service_id: &str,
    endpoint_addr: &EndpointAddr,
    relay_url: Option<String>,
) -> SignedEndpointInfo {
    let pruned_addr = EndpointAddr::new(endpoint_addr.id);
    let endpoint_addr_bytes = serde_json::to_vec(&pruned_addr).unwrap();
    let info = EndpointInfo {
        service_id: service_id.to_string(),
        substrate_id: service_id.to_string(),
        endpoint_type: EndpointType::Substrate,
        nickname: Some("test-node".to_string()),
        mechanisms: vec![EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url }],
        is_private: false,
        ttl: None,
        delegation: None,
    };

    info.sign(identity).unwrap()
}

#[tokio::test]
async fn test_registry_propagation() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();

    // 1. Spawn global registry R
    let mut config_r = SubstrateConfig {
        app_local_data_dir: base_path.join("data_r"),
        app_data_dir: base_path.join("user_data_r"),
        ..Default::default()
    };
    config_r.substrate.enable_bep0044_dht = false;
    config_r.roles.community_registry = Some(ServiceRegistryRole {
        access: AccessControl::String("everyone".to_string()),
        http_bind_address: "127.0.0.1:0".to_string(),
        parent_registry_url: None,
    });
    let mut registry_r = EcosystemRegistry::init(&config_r).await?;
    let r_url = registry_r.bind().await?;
    registry_r.spawn().await?;

    // 2. Spawn private registry Rp pointing to R
    let mut config_rp = SubstrateConfig {
        app_local_data_dir: base_path.join("data_rp"),
        app_data_dir: base_path.join("user_data_rp"),
        ..Default::default()
    };
    config_rp.substrate.enable_bep0044_dht = false;
    config_rp.roles.community_registry = Some(ServiceRegistryRole {
        access: AccessControl::String("everyone".to_string()),
        http_bind_address: "127.0.0.1:0".to_string(),
        parent_registry_url: Some(r_url.clone()),
    });
    let mut registry_rp = EcosystemRegistry::init(&config_rp).await?;
    let rp_url = registry_rp.bind().await?;
    registry_rp.spawn().await?;

    // 3. Register info on Rp
    let identity = Identity::generate()?;
    let did = derive_did_key(&identity.public_key());
    let dummy_secret = SecretKey::generate(&mut rand::rng());
    let dummy_ep = Endpoint::empty_builder().secret_key(dummy_secret).bind().await?;
    let dummy_addr = dummy_ep.addr();

    let signed_info = create_signed_info(&identity, &did, &dummy_addr, None);

    let client = Client::new();
    let res = client.post(format!("{rp_url}/register")).json(&signed_info).send().await?;
    assert!(res.status().is_success());

    // 4. Verify propagation to R
    let mut propagated = false;
    let registry_client = RegistryClient::new(true, Some(r_url.clone()));
    for _ in 0..10 {
        time::sleep(Duration::from_millis(100)).await;
        if let Ok(info) = registry_client.lookup(&did, true).await {
            assert_eq!(info.info.service_id, did);
            propagated = true;
            break;
        }
    }
    assert!(propagated, "Registration failed to propagate to parent registry");

    registry_rp.shutdown().await?;
    registry_r.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_inbound_relay() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();

    // 1. Spawn global registry R
    let mut config_r = SubstrateConfig {
        app_local_data_dir: base_path.join("data_r"),
        app_data_dir: base_path.join("user_data_r"),
        ..Default::default()
    };
    config_r.substrate.enable_bep0044_dht = false;
    config_r.roles.community_registry = Some(ServiceRegistryRole {
        access: AccessControl::String("everyone".to_string()),
        http_bind_address: "127.0.0.1:0".to_string(),
        parent_registry_url: None,
    });
    let mut registry_r = EcosystemRegistry::init(&config_r).await?;
    let r_url = registry_r.bind().await?;
    registry_r.spawn().await?;

    // 2. Spawn Global Coordinator C
    let mut config_c = SubstrateConfig {
        app_local_data_dir: base_path.join("data_c"),
        app_data_dir: base_path.join("user_data_c"),
        ..Default::default()
    };
    config_c.substrate.enable_bep0044_dht = false;
    config_c.roles.coordinator = Some(CoordinatorRole {
        iroh: Some(CoordinatorIrohConfig {
            enable_signalling: true,
            enable_relay: true,
            http_bind_address: "127.0.0.1:0".to_string(),
            quic_bind_address: "127.0.0.1:0".to_string(),
            community_registry_url: Some(r_url.clone()),
            idle_timeout_secs: None,
            share_in_registry: true,
            max_connections: None,
        }),
        ..Default::default()
    });
    let mut c = CoordinatorIroh::init(&config_c).await?;
    let c_info_addr = c.info_addr().unwrap();

    let info_client = Client::new();
    let c_info: CoordinatorInfo =
        info_client.get(format!("http://{c_info_addr}/v1/info")).send().await?.json().await?;
    let c_relay_url = c_info.relay_url.clone().unwrap();

    // 3. Spawn Private Coordinator Cp pointing to C
    let mut config_cp = SubstrateConfig {
        app_local_data_dir: base_path.join("data_cp"),
        app_data_dir: base_path.join("user_data_cp"),
        ..Default::default()
    };
    config_cp.substrate.enable_bep0044_dht = false;
    config_cp.roles.coordinator = Some(CoordinatorRole {
        iroh: Some(CoordinatorIrohConfig {
            enable_signalling: true,
            enable_relay: true,
            http_bind_address: "127.0.0.1:0".to_string(),
            quic_bind_address: "127.0.0.1:0".to_string(),
            community_registry_url: Some(r_url.clone()),
            idle_timeout_secs: None,
            share_in_registry: true,
            max_connections: None,
        }),
        ..Default::default()
    });
    config_cp.parent_coordinator.iroh = Some(IrohParentConfig { url: c_relay_url.clone() });
    let mut cp = CoordinatorIroh::init(&config_cp).await?;
    let cp_info_addr = cp.info_addr().unwrap();

    let cp_info: CoordinatorInfo =
        info_client.get(format!("http://{cp_info_addr}/v1/info")).send().await?.json().await?;

    // 4. Spawn target substrate Sz in private network under Cp
    let identity_z = Identity::generate()?;
    let secret_z_bytes = identity_z.to_bytes();
    let did_z = derive_did_key(&identity_z.public_key());

    let mut config_z = SubstrateConfig {
        app_local_data_dir: base_path.join("data_z"),
        app_data_dir: base_path.join("user_data_z"),
        ..Default::default()
    };
    config_z.substrate.enable_bep0044_dht = false;
    config_z.resolve_paths();
    let data_store_z = registry_store::init_store(&config_z).await?;
    let endpoint_registry_z = EndpointRegistry::new(data_store_z).await?;

    let endpoint_z = SubstrateEndpoint::NativeHostChannel { service_id: did_z.clone() };
    endpoint_registry_z.register(did_z.clone(), "orchestrator".to_string(), endpoint_z).await?;

    // Bind Sz to Iroh so Cp can connect to it (Sz uses Cp's relay url). Built
    // *before* `RouteHandler::init` (M04A Slice A1, §6): the Universal
    // Proxy's outbound remote hop needs a live Iroh endpoint, which
    // `RouteHandler::init` wires into its `ProxyRouter`.
    let mut ep_z_bldr = Endpoint::empty_builder();
    if let Some(relay_url) = cp_info.relay_url.as_ref().and_then(|r| r.parse::<RelayUrl>().ok()) {
        ep_z_bldr = ep_z_bldr.relay_mode(RelayMode::Custom(RelayMap::from(relay_url)));
    }
    let secret_key_z = SecretKey::generate(&mut rand::rng());
    let ep_z = ep_z_bldr.secret_key(secret_key_z).bind().await?;
    ep_z.online().await;

    let deps_z = build_test_route_handler_deps(&config_z, &did_z, &endpoint_registry_z).await?;
    let route_handler_z = RouteHandler::init(
        did_z.clone(),
        &config_z,
        endpoint_registry_z,
        secret_z_bytes,
        Some(ep_z.clone()),
        deps_z,
    )
    .await?;

    let ep_z_addr = ep_z.addr();
    let router_z = Router::builder(ep_z).accept(SYNEROYM_ALPN, route_handler_z).spawn();

    // Register Sz in community registry R
    let signed_info_z =
        create_signed_info(&identity_z, &did_z, &ep_z_addr, cp_info.relay_url.clone());
    let res = info_client.post(format!("{r_url}/register")).json(&signed_info_z).send().await?;
    assert!(res.status().is_success());

    // 5. Connect from a client in public network (under C) to Sz via C -> Cp
    let mut sdk_client = SyneroymClient::new_with_mechanisms(
        did_z.clone(),
        vec![EndpointMechanism::Iroh {
            endpoint_addr_bytes: c_info.endpoint_addr_bytes,
            relay_url: c_info.relay_url,
        }],
    );

    sdk_client.connect().await?;

    // Request readyz!
    let response = sdk_client.request("orchestrator", "readyz", serde_json::json!({})).await?;
    assert_eq!(response.result, serde_json::json!({"status": "ok"}));

    sdk_client.shutdown().await?;
    let _ = router_z.shutdown().await;
    cp.shutdown().await?;
    c.shutdown().await?;
    registry_r.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_outbound_relay() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();

    // 1. Spawn global registry R
    let mut config_r = SubstrateConfig {
        app_local_data_dir: base_path.join("data_r"),
        app_data_dir: base_path.join("user_data_r"),
        ..Default::default()
    };
    config_r.substrate.enable_bep0044_dht = false;
    config_r.roles.community_registry = Some(ServiceRegistryRole {
        access: AccessControl::String("everyone".to_string()),
        http_bind_address: "127.0.0.1:0".to_string(),
        parent_registry_url: None,
    });
    let mut registry_r = EcosystemRegistry::init(&config_r).await?;
    let r_url = registry_r.bind().await?;
    registry_r.spawn().await?;

    // 2. Spawn Global Coordinator C
    let mut config_c = SubstrateConfig {
        app_local_data_dir: base_path.join("data_c"),
        app_data_dir: base_path.join("user_data_c"),
        ..Default::default()
    };
    config_c.substrate.enable_bep0044_dht = false;
    config_c.roles.coordinator = Some(CoordinatorRole {
        iroh: Some(CoordinatorIrohConfig {
            enable_signalling: true,
            enable_relay: true,
            http_bind_address: "127.0.0.1:0".to_string(),
            quic_bind_address: "127.0.0.1:0".to_string(),
            community_registry_url: Some(r_url.clone()),
            idle_timeout_secs: None,
            share_in_registry: true,
            max_connections: None,
        }),
        ..Default::default()
    });
    let mut c = CoordinatorIroh::init(&config_c).await?;
    let c_info_addr = c.info_addr().unwrap();

    let info_client = Client::new();
    let c_info: CoordinatorInfo =
        info_client.get(format!("http://{c_info_addr}/v1/info")).send().await?.json().await?;
    let c_relay_url = c_info.relay_url.clone().unwrap();

    // 3. Spawn Private Coordinator Cp pointing to C
    let mut config_cp = SubstrateConfig {
        app_local_data_dir: base_path.join("data_cp"),
        app_data_dir: base_path.join("user_data_cp"),
        ..Default::default()
    };
    config_cp.substrate.enable_bep0044_dht = false;
    config_cp.roles.coordinator = Some(CoordinatorRole {
        iroh: Some(CoordinatorIrohConfig {
            enable_signalling: true,
            enable_relay: true,
            http_bind_address: "127.0.0.1:0".to_string(),
            quic_bind_address: "127.0.0.1:0".to_string(),
            community_registry_url: Some(r_url.clone()),
            idle_timeout_secs: None,
            share_in_registry: true,
            max_connections: None,
        }),
        ..Default::default()
    });
    config_cp.parent_coordinator.iroh = Some(IrohParentConfig { url: c_relay_url.clone() });
    let mut cp = CoordinatorIroh::init(&config_cp).await?;
    let cp_info_addr = cp.info_addr().unwrap();

    let cp_info: CoordinatorInfo =
        info_client.get(format!("http://{cp_info_addr}/v1/info")).send().await?.json().await?;

    // 4. Spawn target substrate Sx in public network under C
    let identity_x = Identity::generate()?;
    let secret_x_bytes = identity_x.to_bytes();
    let did_x = derive_did_key(&identity_x.public_key());

    let mut config_x = SubstrateConfig {
        app_local_data_dir: base_path.join("data_x"),
        app_data_dir: base_path.join("user_data_x"),
        ..Default::default()
    };
    config_x.substrate.enable_bep0044_dht = false;
    config_x.resolve_paths();
    let data_store_x = registry_store::init_store(&config_x).await?;
    let endpoint_registry_x = EndpointRegistry::new(data_store_x).await?;

    let endpoint_x = SubstrateEndpoint::NativeHostChannel { service_id: did_x.clone() };
    endpoint_registry_x.register(did_x.clone(), "orchestrator".to_string(), endpoint_x).await?;

    // Bind Sx to Iroh so C can connect to it (Sx uses C's relay url). Built
    // *before* `RouteHandler::init` (M04A Slice A1, §6) -- see the matching
    // comment on Sz above.
    let mut ep_x_bldr = Endpoint::empty_builder();
    ep_x_bldr = ep_x_bldr
        .relay_mode(RelayMode::Custom(RelayMap::from(c_relay_url.parse::<RelayUrl>().unwrap())));
    let secret_key_x = SecretKey::generate(&mut rand::rng());
    let ep_x = ep_x_bldr.secret_key(secret_key_x).bind().await?;
    ep_x.online().await;

    let deps_x = build_test_route_handler_deps(&config_x, &did_x, &endpoint_registry_x).await?;
    let route_handler_x = RouteHandler::init(
        did_x.clone(),
        &config_x,
        endpoint_registry_x,
        secret_x_bytes,
        Some(ep_x.clone()),
        deps_x,
    )
    .await?;

    let ep_x_addr = ep_x.addr();
    let router_x = Router::builder(ep_x).accept(SYNEROYM_ALPN, route_handler_x).spawn();

    // Register Sx in community registry R
    let signed_info_x =
        create_signed_info(&identity_x, &did_x, &ep_x_addr, Some(c_relay_url.clone()));
    let res = info_client.post(format!("{r_url}/register")).json(&signed_info_x).send().await?;
    assert!(res.status().is_success());

    // 5. Connect from a client in private network (under Cp) to Sx via Cp -> C
    let mut sdk_client = SyneroymClient::new_with_mechanisms(
        did_x.clone(),
        vec![EndpointMechanism::Iroh {
            endpoint_addr_bytes: cp_info.endpoint_addr_bytes,
            relay_url: cp_info.relay_url,
        }],
    );

    sdk_client.connect().await?;

    // Request readyz!
    let response = sdk_client.request("orchestrator", "readyz", serde_json::json!({})).await?;
    assert_eq!(response.result, serde_json::json!({"status": "ok"}));

    sdk_client.shutdown().await?;
    let _ = router_x.shutdown().await;
    cp.shutdown().await?;
    c.shutdown().await?;
    registry_r.shutdown().await?;
    Ok(())
}

/// M04A Slice A1 (§11.6, reference scenario step 20): a real cross-node
/// Universal Proxy call between two full substrate nodes. Unlike
/// `test_inbound_relay`/`test_outbound_relay`, no coordinator/relay
/// infrastructure is needed here -- Sx and Sz both bind direct-address-only
/// Iroh endpoints (same machine, same process) and a lightweight HTTP
/// community registry is enough for Sx's `ProxyRouter` to resolve Sz's
/// address. Sx hosts the `proxy-test` component (imports
/// `syneroym:proxy/proxy`); Sz hosts `greeter`. Driving `call-peer` on Sx
/// exercises the full remote hop end to end -- §6's outbound-endpoint fix,
/// §5.5's `IrohHop`/retry loop, and proof/identity forwarding -- not just
/// the in-process `ProxyRouter::invoke` unit tests (`crates/router/src/
/// proxy.rs`) or the same-node guest-to-guest test (`proxy_dispatch.rs`).
#[tokio::test]
async fn test_cross_node_proxy_call() -> Result<()> {
    let Ok(greeter_bytes) = std::fs::read(test_constants::greeter_wasm_path()) else {
        eprintln!("skipping: greeter wasm artifact not built");
        return Ok(());
    };
    let Ok(proxy_test_bytes) = std::fs::read(test_constants::proxy_test_wasm_path()) else {
        eprintln!("skipping: proxy-test wasm artifact not built");
        return Ok(());
    };

    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();

    // 1. A lightweight HTTP community registry (no DHT) -- just enough for Sx's
    //    ProxyRouter to resolve Sz's Iroh address.
    let mut config_r = SubstrateConfig {
        app_local_data_dir: base_path.join("data_r"),
        app_data_dir: base_path.join("user_data_r"),
        ..Default::default()
    };
    config_r.substrate.enable_bep0044_dht = false;
    config_r.roles.community_registry = Some(ServiceRegistryRole {
        access: AccessControl::String("everyone".to_string()),
        http_bind_address: "127.0.0.1:0".to_string(),
        parent_registry_url: None,
    });
    let mut registry_r = EcosystemRegistry::init(&config_r).await?;
    let r_url = registry_r.bind().await?;
    registry_r.spawn().await?;

    // 2. Node Sz: the greeter callee. Direct-address-only Iroh endpoint (no relay
    //    -- both nodes are local, direct addresses suffice).
    let identity_z = Identity::generate()?;
    let secret_z_bytes = identity_z.to_bytes();
    let did_z = derive_did_key(&identity_z.public_key());

    let mut config_z = SubstrateConfig {
        app_local_data_dir: base_path.join("data_z"),
        app_data_dir: base_path.join("user_data_z"),
        ..Default::default()
    };
    config_z.substrate.enable_bep0044_dht = false;
    config_z.resolve_paths();
    let data_store_z = registry_store::init_store(&config_z).await?;
    let endpoint_registry_z = EndpointRegistry::new(data_store_z).await?;

    // No `.online()` wait here (unlike `test_inbound_relay`/`test_outbound_relay`
    // above): those configure a relay and `.online()` waits for *both* a
    // relay connection *and* a local address; these two endpoints have no
    // relay at all (direct addresses only, same machine), so `.online()`
    // would wait indefinitely for the relay half of that condition.
    // `wait_for_local_addr` polls `.addr()` until it has at least one
    // direct address instead.
    let ep_z =
        Endpoint::empty_builder().secret_key(SecretKey::from_bytes(&secret_z_bytes)).bind().await?;
    let ep_z_addr = wait_for_local_addr(&ep_z).await;

    let deps_z = build_test_route_handler_deps(&config_z, &did_z, &endpoint_registry_z).await?;
    let app_sandbox_engine_z = deps_z.app_sandbox_engine.clone();
    let route_handler_z = RouteHandler::init(
        did_z.clone(),
        &config_z,
        endpoint_registry_z.clone(),
        secret_z_bytes,
        Some(ep_z.clone()),
        deps_z,
    )
    .await?;
    let router_z = Router::builder(ep_z).accept(SYNEROYM_ALPN, route_handler_z).spawn();

    // Deployed *under did_z itself*, not a separate service id: the proxy
    // target (`service = did_z`) is what the community registry resolves to
    // Sz's Iroh address, so it must also be the id Sz's own local
    // `EndpointRegistry`/`AppSandboxEngine` resolve the WASM component
    // under -- otherwise Sz would successfully accept the connection but
    // find no local route for it once it arrives.
    app_sandbox_engine_z.deploy_wasm(&did_z, &wasm_deploy_manifest(greeter_bytes)).await.unwrap();
    endpoint_registry_z
        .register(
            did_z.clone(),
            test_constants::GREETER_INTERFACE_NAME.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: did_z.clone() },
        )
        .await?;

    // did_z itself must also resolve (the proxy target is did_z, and
    // `greeter-svc` is deployed *on* the did_z node, addressed through the
    // registry's did_z -> Iroh-mechanism mapping).
    let signed_info_z = create_signed_info_with_full_addr(&identity_z, &did_z, &ep_z_addr);
    let info_client = Client::new();
    let res = info_client.post(format!("{r_url}/register")).json(&signed_info_z).send().await?;
    assert!(res.status().is_success());

    // 3. Node Sx: the proxy-test caller. `registry_url` points at R so its
    //    `ProxyRouter`'s outbound remote hop can resolve did_z.
    let identity_x = Identity::generate()?;
    let secret_x_bytes = identity_x.to_bytes();
    let did_x = derive_did_key(&identity_x.public_key());

    let mut config_x = SubstrateConfig {
        app_local_data_dir: base_path.join("data_x"),
        app_data_dir: base_path.join("user_data_x"),
        ..Default::default()
    };
    config_x.substrate.enable_bep0044_dht = false;
    config_x.substrate.registry_url = Some(r_url.clone());
    config_x.resolve_paths();
    let data_store_x = registry_store::init_store(&config_x).await?;
    let endpoint_registry_x = EndpointRegistry::new(data_store_x).await?;

    let ep_x =
        Endpoint::empty_builder().secret_key(SecretKey::from_bytes(&secret_x_bytes)).bind().await?;

    let deps_x = build_test_route_handler_deps(&config_x, &did_x, &endpoint_registry_x).await?;
    let app_sandbox_engine_x = deps_x.app_sandbox_engine.clone();
    let route_handler_x = RouteHandler::init(
        did_x.clone(),
        &config_x,
        endpoint_registry_x.clone(),
        secret_x_bytes,
        Some(ep_x.clone()),
        deps_x,
    )
    .await?;

    app_sandbox_engine_x
        .deploy_wasm("proxy-caller", &wasm_deploy_manifest(proxy_test_bytes))
        .await
        .unwrap();
    endpoint_registry_x
        .register(
            "proxy-caller".to_string(),
            test_constants::PROXY_TEST_DRIVER_INTERFACE.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "proxy-caller".to_string() },
        )
        .await?;

    // 4. Drive `call-peer` on Sx, targeting did_z's greeter -- this is the
    //    cross-node hop: Sx's local registry has no entry for did_z, so
    //    `ProxyRouter::invoke` falls to `invoke_remote`, resolves did_z via the
    //    HTTP registry, and connects `ep_x` directly to `ep_z_addr`.
    let pipeline = RoutePipeline {
        encryption: EncryptionStage::None,
        transport: TransportStage::Binary,
        adaptation: AdaptationStage::JsonRpcToWasm,
        service: ServiceStage::WasmComponent { service_id: "proxy-caller".to_string() },
    };
    let preamble = RoutePreamble {
        transport: RouteTransport::Binary,
        protocol: RouteProtocol::JsonRpc,
        interface: test_constants::PROXY_TEST_DRIVER_INTERFACE.to_string(),
        service_id: "proxy-caller".to_string(),
        enc: None,
        pubkey: None,
        delegation: None,
        ucan: None,
        dir: None,
    };
    let body = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "call-peer",
        "params": {
            "service": did_z,
            "interface": test_constants::GREETER_INTERFACE_NAME,
            "method": "greet",
            "params": "[\"Cross-Node\"]",
        },
        "id": 1,
    }))?;

    let response_bytes =
        route_handler_x.dispatch_json_rpc_once(&pipeline, &preamble, None, &body).await?;
    let response: serde_json::Value = serde_json::from_slice(&response_bytes)?;
    assert!(response.get("error").is_none(), "cross-node call-peer failed: {response:?}");
    let result = response.get("result").and_then(serde_json::Value::as_str).unwrap_or_default();
    assert!(
        result.contains("Hello, Cross-Node!"),
        "expected did_z's greeter response, got: {response:?}"
    );

    let _ = router_z.shutdown().await;
    ep_x.close().await;
    registry_r.shutdown().await?;
    Ok(())
}

/// A `NativeService` test double that records the `caller_did` of every
/// invocation it receives, for asserting exactly what identity a dispatch
/// carried.
#[derive(Debug, Default)]
struct CapturingNativeService {
    captured_caller_did: Mutex<Option<String>>,
}

#[async_trait]
impl NativeService for CapturingNativeService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        *self.captured_caller_did.lock().unwrap() = Some(invocation.caller.caller_did.clone());
        Ok(NativeResponse { payload: serde_json::json!({"status": "ok"}) })
    }
}

/// M04A Slice A1 post-commit review finding F3: `test_cross_node_proxy_call`
/// above only drives a guest -> WASM greeter call, where the callee runs as
/// `service_system` and native-capability identity never crosses -- the
/// full loop (`invoke_remote_at` builds the outbound preamble from a real
/// `CallerProof` -> destination `verify_preamble` re-verifies the
/// delegation cert -> `build_caller` -> native dispatch with the re-verified
/// caller) had no integration coverage. This exercises that loop directly
/// (via `AppSandboxEngine::service_proxy`, bypassing the WASM guest path,
/// since a guest never carries a proof in A1 -- see `CallOrigin::Native`
/// callers in `syneroym_router::proxy`) and asserts Sz's `NativeService`
/// sees the re-verified master DID, not the temporary session key or the
/// forwarding node's own identity.
#[tokio::test]
async fn test_cross_node_native_capability_identity_forwarding() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();

    // 1. Community registry -- also where Sz resolves the caller's master anchor to
    //    re-verify the delegation certificate.
    let mut config_r = SubstrateConfig {
        app_local_data_dir: base_path.join("data_r"),
        app_data_dir: base_path.join("user_data_r"),
        ..Default::default()
    };
    config_r.substrate.enable_bep0044_dht = false;
    config_r.roles.community_registry = Some(ServiceRegistryRole {
        access: AccessControl::String("everyone".to_string()),
        http_bind_address: "127.0.0.1:0".to_string(),
        parent_registry_url: None,
    });
    let mut registry_r = EcosystemRegistry::init(&config_r).await?;
    let r_url = registry_r.bind().await?;
    registry_r.spawn().await?;

    // 2. Node Sz: the native-capability callee.
    let identity_z = Identity::generate()?;
    let secret_z_bytes = identity_z.to_bytes();
    let did_z = derive_did_key(&identity_z.public_key());

    let mut config_z = SubstrateConfig {
        app_local_data_dir: base_path.join("data_z"),
        app_data_dir: base_path.join("user_data_z"),
        ..Default::default()
    };
    config_z.substrate.enable_bep0044_dht = false;
    // Sz's own `RegistryClient` (built inside `RouteHandler::init`) needs
    // this to resolve the caller's master anchor during handshake
    // verification -- unlike the greeter test above, this test's caller
    // presents a real delegation cert, not a proof-less/self-signed key.
    config_z.substrate.registry_url = Some(r_url.clone());
    config_z.resolve_paths();
    let data_store_z = registry_store::init_store(&config_z).await?;
    let endpoint_registry_z = EndpointRegistry::new(data_store_z).await?;

    let ep_z =
        Endpoint::empty_builder().secret_key(SecretKey::from_bytes(&secret_z_bytes)).bind().await?;
    let ep_z_addr = wait_for_local_addr(&ep_z).await;

    let deps_z = build_test_route_handler_deps(&config_z, &did_z, &endpoint_registry_z).await?;
    // Grab the (shared) dispatch table before `RouteHandler::init` consumes
    // `deps_z` -- `RouteHandler::init` registers `control_plane_service`
    // under `did_z` itself, so the capturing double below is registered
    // under a distinct key to avoid colliding with it.
    let native_dispatch_z = deps_z.native_dispatch.clone();
    let capturing_service = Arc::new(CapturingNativeService::default());
    native_dispatch_z.insert(
        "data-layer-svc-z".to_string(),
        capturing_service.clone() as Arc<dyn NativeService>,
    );
    endpoint_registry_z
        .register(
            did_z.clone(),
            "data-layer".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "data-layer-svc-z".to_string() },
        )
        .await?;

    let route_handler_z = RouteHandler::init(
        did_z.clone(),
        &config_z,
        endpoint_registry_z.clone(),
        secret_z_bytes,
        Some(ep_z.clone()),
        deps_z,
    )
    .await?;
    let router_z = Router::builder(ep_z).accept(SYNEROYM_ALPN, route_handler_z).spawn();

    let signed_info_z = create_signed_info_with_full_addr(&identity_z, &did_z, &ep_z_addr);
    let info_client = Client::new();
    let res = info_client.post(format!("{r_url}/register")).json(&signed_info_z).send().await?;
    assert!(res.status().is_success());

    // 3. Node Sx: the caller. It only needs `RouteHandler::init` to wire up a
    //    `ProxyRouter` (reachable via `AppSandboxEngine::service_proxy`) -- it
    //    never receives inbound connections in this test, so no accept loop is
    //    spawned for it.
    let identity_x = Identity::generate()?;
    let secret_x_bytes = identity_x.to_bytes();
    let did_x = derive_did_key(&identity_x.public_key());

    let mut config_x = SubstrateConfig {
        app_local_data_dir: base_path.join("data_x"),
        app_data_dir: base_path.join("user_data_x"),
        ..Default::default()
    };
    config_x.substrate.enable_bep0044_dht = false;
    config_x.substrate.registry_url = Some(r_url.clone());
    config_x.resolve_paths();
    let data_store_x = registry_store::init_store(&config_x).await?;
    let endpoint_registry_x = EndpointRegistry::new(data_store_x).await?;

    let ep_x =
        Endpoint::empty_builder().secret_key(SecretKey::from_bytes(&secret_x_bytes)).bind().await?;

    let deps_x = build_test_route_handler_deps(&config_x, &did_x, &endpoint_registry_x).await?;
    let app_sandbox_engine_x = deps_x.app_sandbox_engine.clone();
    let route_handler_x = RouteHandler::init(
        did_x.clone(),
        &config_x,
        endpoint_registry_x.clone(),
        secret_x_bytes,
        Some(ep_x.clone()),
        deps_x,
    )
    .await?;

    let service_proxy = app_sandbox_engine_x
        .service_proxy
        .get()
        .expect("RouteHandler::init sets AppSandboxEngine::service_proxy")
        .upgrade()
        .expect("route_handler_x keeps the ProxyRouter alive");

    // 4. A real caller proof: a temporary key delegated by a master identity, with
    //    the master's anchor published to the same registry Sz resolves against.
    let master = Identity::generate()?;
    let master_did = derive_did_key(&master.public_key());
    let temp = Identity::generate()?;
    let temp_pubkey_hex = hex::encode(temp.public_key().to_bytes());
    let cert =
        DelegationCertificate::issue(&master, temp.public_key(), 3600, "routing".to_string())?;

    let registry_client = RegistryClient::new(false, Some(r_url.clone()));
    registry_client.publish_master_anchor(&master_did, vec![], None, &master, false).await?;

    let req = ProxyRequest {
        target_service: did_z.clone(),
        interface: "data-layer".to_string(),
        method: "query".to_string(),
        params: serde_json::json!({}),
        caller: CallerContext {
            caller_did: master_did.clone(),
            app_instance: None,
            session: SessionContext::default(),
            auth: AuthLevel::Delegated,
            proof: Some(CallerProof {
                pubkey_hex: temp_pubkey_hex,
                delegation_json: Some(cert.to_json()?),
            }),
        },
        origin: CallOrigin::Native,
        protocol: ProxyProtocol::JsonRpcV1,
        idempotent: false,
        timeout: Some(Duration::from_secs(5)),
    };

    service_proxy.invoke(req).await.map_err(|e| anyhow::anyhow!("proxy call failed: {e}"))?;

    assert_eq!(
        capturing_service.captured_caller_did.lock().unwrap().as_deref(),
        Some(master_did.as_str()),
        "Sz's NativeService should see the re-verified master DID, not the temporary session key \
         or the forwarding node's own identity"
    );

    let _ = route_handler_x;
    let _ = router_z.shutdown().await;
    ep_x.close().await;
    registry_r.shutdown().await?;
    Ok(())
}
