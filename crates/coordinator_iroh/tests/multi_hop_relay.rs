//! Integration tests for the multi-hop relay functionality
//!
//! Simulates the scenario described in the multi-hop-relay-scenario.md
//! which organizes relays, registries, substrates across network boundaries in
//! a hierarchy and tests bidirectional e2e connectivity between clients and
//! substrates across networks.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::time::Duration;

use anyhow::Result;
use iroh::{Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl, SecretKey, protocol::Router};
use reqwest::Client;
use syneroym_community_registry::EcosystemRegistry;
use syneroym_coordinator_iroh::{CoordinatorInfo, CoordinatorIroh};
use syneroym_core::{
    config::{
        AccessControl, CoordinatorIrohConfig, CoordinatorRole, IrohParentConfig,
        ServiceRegistryRole, SubstrateConfig,
    },
    dht_registry::{
        EndpointInfo, EndpointMechanism, EndpointType, RegistryClient, SignedEndpointInfo,
    },
    local_registry::{EndpointRegistry, SubstrateEndpoint},
};
use syneroym_data_layer::registry_store;
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_router::{RouteHandler, SYNEROYM_ALPN};
use syneroym_sdk::SyneroymClient;
use tokio::time;

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
    config_z.resolve_paths();
    let data_store_z = registry_store::init_store(&config_z).await?;
    let endpoint_registry_z = EndpointRegistry::new(data_store_z).await?;

    let endpoint_z = SubstrateEndpoint::NativeHostChannel { service_id: did_z.clone() };
    endpoint_registry_z.register(did_z.clone(), "orchestrator".to_string(), endpoint_z).await?;

    let route_handler_z =
        RouteHandler::init(did_z.clone(), &config_z, endpoint_registry_z, secret_z_bytes).await?;

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
    config_x.resolve_paths();
    let data_store_x = registry_store::init_store(&config_x).await?;
    let endpoint_registry_x = EndpointRegistry::new(data_store_x).await?;

    let endpoint_x = SubstrateEndpoint::NativeHostChannel { service_id: did_x.clone() };
    endpoint_registry_x.register(did_x.clone(), "orchestrator".to_string(), endpoint_x).await?;

    let route_handler_x =
        RouteHandler::init(did_x.clone(), &config_x, endpoint_registry_x, secret_x_bytes).await?;

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
