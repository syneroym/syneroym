#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use anyhow::Result;
use iroh::{Endpoint, RelayMap, RelayMode, RelayUrl, SecretKey};
use std::sync::Arc;
use std::time::Duration;
use syneroym_community_registry::EcosystemRegistry;
use syneroym_coordinator_iroh::{CoordinatorInfo, CoordinatorIroh};
use syneroym_core::community_registry::{
    EndpointInfo, EndpointMechanism, EndpointType, SignedEndpointInfo,
};
use syneroym_core::config::{
    CoordinatorIrohConfig, CoordinatorRole, ServiceRegistryRole, SubstrateConfig,
};
use syneroym_identity::Identity;
use syneroym_identity::substrate::derive_did_key;
use syneroym_router::RouteHandler;

#[derive(Debug)]
struct EchoService;

#[async_trait::async_trait]
impl syneroym_rpc::NativeService for EchoService {
    async fn dispatch(
        &self,
        invocation: syneroym_rpc::NativeInvocation,
    ) -> syneroym_rpc::RpcResult<syneroym_rpc::NativeResponse> {
        Ok(syneroym_rpc::NativeResponse { payload: invocation.params })
    }
}

fn create_signed_info(
    identity: &Identity,
    service_id: &str,
    endpoint_addr: &iroh::EndpointAddr,
) -> SignedEndpointInfo {
    let endpoint_addr_bytes = serde_json::to_vec(endpoint_addr).unwrap();
    let info = EndpointInfo {
        service_id: service_id.to_string(),
        substrate_id: service_id.to_string(),
        endpoint_type: EndpointType::Substrate,
        nickname: Some("test-node".to_string()),
        mechanisms: vec![EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url: None }],
        is_private: false,
    };

    let signature_z32 = identity.sign_json(&serde_json::to_value(&info).unwrap()).unwrap();
    SignedEndpointInfo { info, signature: signature_z32 }
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
        access: syneroym_core::config::AccessControl::String("everyone".to_string()),
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
        access: syneroym_core::config::AccessControl::String("everyone".to_string()),
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
    let dummy_ep =
        Endpoint::builder(iroh::endpoint::presets::N0).secret_key(dummy_secret).bind().await?;
    let dummy_addr = dummy_ep.addr();

    let signed_info = create_signed_info(&identity, &did, &dummy_addr);

    let client = reqwest::Client::new();
    let res = client.post(format!("{rp_url}/register")).json(&signed_info).send().await?;
    assert!(res.status().is_success());

    // 4. Verify propagation to R
    let mut propagated = false;
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Ok(info) =
            syneroym_core::community_registry::RegistryClient::lookup(&r_url, &did, true).await
        {
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
async fn test_ax_to_az_inbound_relay() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();

    // 1. Spawn global registry R
    let mut config_r = SubstrateConfig {
        app_local_data_dir: base_path.join("data_r"),
        app_data_dir: base_path.join("user_data_r"),
        ..Default::default()
    };
    config_r.roles.community_registry = Some(ServiceRegistryRole {
        access: syneroym_core::config::AccessControl::String("everyone".to_string()),
        http_bind_address: "127.0.0.1:0".to_string(),
        parent_registry_url: None,
    });
    let mut registry_r = EcosystemRegistry::init(&config_r).await?;
    let r_url = registry_r.bind().await?;
    registry_r.spawn().await?;

    // 2. Spawn Coordinator Cp
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
            share_in_registry: true,
        }),
        ..Default::default()
    });
    let mut cp = CoordinatorIroh::init(&config_cp).await?;
    let cp_info_addr = cp.info_addr().unwrap();
    let cp_ep_addr = cp.endpoint_addr().unwrap();

    // 3. Spawn target substrate Sz in private network
    // Sz has its target service Az deployed locally.
    let identity_z = Identity::generate()?;
    let secret_z_bytes = identity_z.to_bytes();
    let did_z = derive_did_key(&identity_z.public_key());

    // Create RouteHandler for Sz with EchoService
    let mut config_z = SubstrateConfig {
        app_local_data_dir: base_path.join("data_z"),
        app_data_dir: base_path.join("user_data_z"),
        ..Default::default()
    };
    config_z.resolve_paths();
    let data_store_z = syneroym_core::storage::init_store(&config_z).await?;
    let endpoint_registry_z = syneroym_core::registry::EndpointRegistry::new(data_store_z).await?;

    let endpoint_z =
        syneroym_core::registry::SubstrateEndpoint::NativeHostChannel { service_id: did_z.clone() };
    endpoint_registry_z.register(did_z.clone(), "echo".to_string(), endpoint_z).await?;

    let route_handler_z =
        RouteHandler::init(did_z.clone(), &config_z, endpoint_registry_z, secret_z_bytes).await?;
    route_handler_z.register_native_service(did_z.clone(), Arc::new(EchoService));

    // Fetch Cp's info early so we can get its dynamically bound relay_url!
    let cp_info_client = reqwest::Client::new();
    let cp_info: CoordinatorInfo =
        cp_info_client.get(format!("http://{cp_info_addr}/v1/info")).send().await?.json().await?;

    let cp_endpoint_addr: iroh::EndpointAddr =
        serde_json::from_slice(&cp_info.endpoint_addr_bytes)?;
    assert_eq!(cp_endpoint_addr.id, cp_ep_addr.id);

    // Bind Sz to Iroh so Cp can connect to it
    let mut ep_z_bldr = Endpoint::builder(iroh::endpoint::presets::N0);
    if let Some(relay_url) = cp_info.relay_url.as_ref().and_then(|r| r.parse::<RelayUrl>().ok()) {
        ep_z_bldr = ep_z_bldr.relay_mode(RelayMode::Custom(RelayMap::from(relay_url)));
    }
    let secret_key_z = SecretKey::generate(&mut rand::rng());
    let ep_z = ep_z_bldr.secret_key(secret_key_z).bind().await?;
    ep_z.online().await;

    let ep_z_addr = ep_z.addr();
    let router_z = iroh::protocol::Router::builder(ep_z)
        .accept(syneroym_router::SYNEROYM_ALPN, route_handler_z)
        .spawn();

    // Register Sz in community registry R
    let signed_info_z = create_signed_info(&identity_z, &did_z, &ep_z_addr);
    let client = reqwest::Client::new();
    let res = client.post(format!("{r_url}/register")).json(&signed_info_z).send().await?;
    assert!(res.status().is_success());

    // 4. Connect from Ax (a client in public network) to Az via Cp

    // Ax starts a client targeting Az, but connects via Cp!
    let mut sdk_client = syneroym_sdk::SyneroymClient::new_with_mechanisms(
        did_z.clone(),
        vec![EndpointMechanism::Iroh {
            endpoint_addr_bytes: cp_info.endpoint_addr_bytes,
            relay_url: cp_info.relay_url,
        }],
    );

    sdk_client.connect().await?;

    // Request echo!
    let response = sdk_client
        .request("echo", "echo_method", serde_json::json!({"message": "Hello Multi-Hop Relay!"}))
        .await?;
    assert_eq!(response.result, serde_json::json!({"message": "Hello Multi-Hop Relay!"}));

    sdk_client.shutdown().await?;
    let _ = router_z.shutdown().await;
    cp.shutdown().await?;
    registry_r.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_az_to_ax_outbound_relay() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();

    // 1. Spawn global registry R
    let mut config_r = SubstrateConfig {
        app_local_data_dir: base_path.join("data_r"),
        app_data_dir: base_path.join("user_data_r"),
        ..Default::default()
    };
    config_r.roles.community_registry = Some(ServiceRegistryRole {
        access: syneroym_core::config::AccessControl::String("everyone".to_string()),
        http_bind_address: "127.0.0.1:0".to_string(),
        parent_registry_url: None,
    });
    let mut registry_r = EcosystemRegistry::init(&config_r).await?;
    let r_url = registry_r.bind().await?;
    registry_r.spawn().await?;

    // 2. Spawn Coordinator Cp
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
            share_in_registry: true,
        }),
        ..Default::default()
    });
    let mut cp = CoordinatorIroh::init(&config_cp).await?;
    let cp_info_addr = cp.info_addr().unwrap();

    // 3. Spawn target substrate Sx in public network
    // Sx has its target service Ax deployed locally.
    let identity_x = Identity::generate()?;
    let secret_x_bytes = identity_x.to_bytes();
    let did_x = derive_did_key(&identity_x.public_key());

    // Create RouteHandler for Sx with EchoService
    let mut config_x = SubstrateConfig {
        app_local_data_dir: base_path.join("data_x"),
        app_data_dir: base_path.join("user_data_x"),
        ..Default::default()
    };
    config_x.resolve_paths();
    let data_store_x = syneroym_core::storage::init_store(&config_x).await?;
    let endpoint_registry_x = syneroym_core::registry::EndpointRegistry::new(data_store_x).await?;

    let endpoint_x =
        syneroym_core::registry::SubstrateEndpoint::NativeHostChannel { service_id: did_x.clone() };
    endpoint_registry_x.register(did_x.clone(), "echo".to_string(), endpoint_x).await?;

    let route_handler_x =
        RouteHandler::init(did_x.clone(), &config_x, endpoint_registry_x, secret_x_bytes).await?;
    route_handler_x.register_native_service(did_x.clone(), Arc::new(EchoService));

    // Fetch Cp's info early so we can get its dynamically bound relay_url!
    let cp_info_client = reqwest::Client::new();
    let cp_info: CoordinatorInfo =
        cp_info_client.get(format!("http://{cp_info_addr}/v1/info")).send().await?.json().await?;

    // Bind Sx to Iroh so Cp can connect to it
    let mut ep_x_bldr = Endpoint::builder(iroh::endpoint::presets::N0);
    if let Some(relay_url) = cp_info.relay_url.as_ref().and_then(|r| r.parse::<RelayUrl>().ok()) {
        ep_x_bldr = ep_x_bldr.relay_mode(RelayMode::Custom(RelayMap::from(relay_url)));
    }
    let secret_key_x = SecretKey::generate(&mut rand::rng());
    let ep_x = ep_x_bldr.secret_key(secret_key_x).bind().await?;
    ep_x.online().await;

    let ep_x_addr = ep_x.addr();
    let router_x = iroh::protocol::Router::builder(ep_x)
        .accept(syneroym_router::SYNEROYM_ALPN, route_handler_x)
        .spawn();

    // Register Sx in community registry R
    let signed_info_x = create_signed_info(&identity_x, &did_x, &ep_x_addr);
    let client = reqwest::Client::new();
    let res = client.post(format!("{r_url}/register")).json(&signed_info_x).send().await?;
    assert!(res.status().is_success());

    // 4. Connect from Az (on Sz in private network) to Ax via Cp
    // Sz discovers Cp via HTTP discovery on /v1/info.

    // Az starts a client targeting Ax, but connects via Cp!
    let mut sdk_client = syneroym_sdk::SyneroymClient::new_with_mechanisms(
        did_x.clone(),
        vec![EndpointMechanism::Iroh {
            endpoint_addr_bytes: cp_info.endpoint_addr_bytes,
            relay_url: cp_info.relay_url,
        }],
    );

    sdk_client.connect().await?;

    // Request echo!
    let response = sdk_client
        .request(
            "echo",
            "echo_method",
            serde_json::json!({"message": "Hello Outbound Multi-Hop Relay!"}),
        )
        .await?;
    assert_eq!(response.result, serde_json::json!({"message": "Hello Outbound Multi-Hop Relay!"}));

    sdk_client.shutdown().await?;
    let _ = router_x.shutdown().await;
    cp.shutdown().await?;
    registry_r.shutdown().await?;
    Ok(())
}
