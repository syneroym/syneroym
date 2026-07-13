//! Integration tests for the multi-hop relay functionality
//!
//! Simulates the scenario described in the multi-hop-relay-scenario.md
//! which organizes relays, registries, substrates across network boundaries in
//! a hierarchy and tests bidirectional e2e connectivity between clients and
//! substrates across networks.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::{sync::Arc, time::Duration};

use anyhow::Result;
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
};
use syneroym_data_blob::ObjectStoreBlobProvider;
use syneroym_data_db::{SqliteStorageProvider, registry_store, traits::StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_router::{RouteHandler, RouteHandlerDeps, SYNEROYM_ALPN};
use syneroym_rpc::NativeDispatchRegistry;
use syneroym_sandbox_podman::ContainerEngine;
use syneroym_sandbox_wasm::AppSandboxEngine;
use syneroym_sdk::SyneroymClient;
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

    let deps_z = build_test_route_handler_deps(&config_z, &did_z, &endpoint_registry_z).await?;
    let route_handler_z =
        RouteHandler::init(did_z.clone(), &config_z, endpoint_registry_z, secret_z_bytes, deps_z)
            .await?;

    // Bind Sz to Iroh so Cp can connect to it (Sz uses Cp's relay url)
    let mut ep_z_bldr = Endpoint::empty_builder();
    if let Some(relay_url) = cp_info.relay_url.as_ref().and_then(|r| r.parse::<RelayUrl>().ok()) {
        ep_z_bldr = ep_z_bldr.relay_mode(RelayMode::Custom(RelayMap::from(relay_url)));
    }
    let secret_key_z = SecretKey::generate(&mut rand::rng());
    let ep_z = ep_z_bldr.secret_key(secret_key_z).bind().await?;
    ep_z.online().await;

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

    let deps_x = build_test_route_handler_deps(&config_x, &did_x, &endpoint_registry_x).await?;
    let route_handler_x =
        RouteHandler::init(did_x.clone(), &config_x, endpoint_registry_x, secret_x_bytes, deps_x)
            .await?;

    // Bind Sx to Iroh so C can connect to it (Sx uses C's relay url)
    let mut ep_x_bldr = Endpoint::empty_builder();
    ep_x_bldr = ep_x_bldr
        .relay_mode(RelayMode::Custom(RelayMap::from(c_relay_url.parse::<RelayUrl>().unwrap())));
    let secret_key_x = SecretKey::generate(&mut rand::rng());
    let ep_x = ep_x_bldr.secret_key(secret_key_x).bind().await?;
    ep_x.online().await;

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
