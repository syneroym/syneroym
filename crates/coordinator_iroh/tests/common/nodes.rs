//! Shared multi-node test helpers for `coordinator_iroh` integration tests.

use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use iroh::{Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl, SecretKey, protocol::Router};
use reqwest::Client;
use syneroym_control_plane::ControlPlaneService;
use syneroym_coordinator_iroh::CoordinatorIroh;
use syneroym_core::{
    asset_manifest::AssetRegistry,
    config::{CoordinatorIrohConfig, CoordinatorRole, SubstrateConfig},
    coordinator_info::{CoordinatorInfo, INFO_PATH},
    dht_registry::{
        EndpointInfo, EndpointMechanism, EndpointType, RegistryClient, SignedEndpointInfo,
    },
    http_routes::HttpRouteRegistry,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
};
use syneroym_data_blob::ObjectStoreBlobProvider;
use syneroym_data_db::{SqliteStorageProvider, registry_store, traits::StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_identity::{DelegationCertificate, Identity, substrate::derive_did_key};
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_router::{
    RouteHandler, RouteHandlerDeps, RoutePreamble, RouteProtocol, RouteTransport, SYNEROYM_ALPN,
};
use syneroym_rpc::{
    AuthLevel, CallOrigin, CallerContext, NativeDispatchRegistry, NativeInvocation, NativeResponse,
    NativeService, ProxyProtocol, ProxyRequest, RpcResult, ServiceProxy, SessionContext,
};
use syneroym_sandbox_podman::ContainerEngine;
use syneroym_sandbox_wasm::AppSandboxEngine;
use syneroym_sdk::SyneroymClient;
use tokio::time;

use super::{init_coordinator, spawn_registry, wait_for_relay};

pub async fn build_test_route_handler_deps(
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
            syneroym_app_orchestration::empty_resolver(),
        )
        .await?,
    );
    app_sandbox_engine
        .self_weak
        .set(Arc::downgrade(&app_sandbox_engine))
        .map_err(|_| anyhow::anyhow!("AppSandboxEngine::self_weak set more than once"))?;
    let websocket_senders = syneroym_rpc::WebSocketSenders::new();
    app_sandbox_engine
        .websocket_senders
        .set(websocket_senders.clone())
        .map_err(|_| anyhow::anyhow!("AppSandboxEngine::websocket_senders set more than once"))?;

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
    let assets: AssetRegistry = Arc::new(DashMap::new());

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
        assets.clone(),
        Arc::new(Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await?;
    let control_plane_service = Arc::new(control_plane_service);

    Ok(RouteHandlerDeps {
        logical_resolver: syneroym_app_orchestration::empty_resolver(),
        key_store,
        storage_provider,
        app_sandbox_engine,
        messaging_broker,
        native_dispatch,
        native_http: Arc::new(DashMap::new()),
        websocket_senders,
        http_routes,
        assets,
        sse_permits: control_plane_service.sse_permits(),
        control_plane_service: control_plane_service.clone(),
        control_plane: Some(control_plane_service),
        session_revocation: None,
    })
}

pub fn create_signed_info_with_full_addr(
    identity: &Identity,
    service_id: &str,
    endpoint_addr: &EndpointAddr,
) -> SignedEndpointInfo {
    let pruned_addrs: BTreeSet<_> = endpoint_addr.addrs.iter().take(2).cloned().collect();
    let pruned = EndpointAddr { id: endpoint_addr.id, addrs: pruned_addrs };
    let endpoint_addr_bytes = serde_json::to_vec(&pruned).unwrap();
    let info = EndpointInfo {
        service_id: service_id.to_string(),
        substrate_id: service_id.to_string(),
        endpoint_type: EndpointType::Substrate,
        nickname: Some("test-node".to_string()),
        mechanisms: vec![EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url: None }],
        is_private: false,
        ttl: None,
        not_after: u64::MAX / 2,
        generation: 0,
        mailbox: None,
    };
    info.sign(identity).unwrap()
}

pub async fn wait_for_local_addr(ep: &Endpoint) -> EndpointAddr {
    for _ in 0..50 {
        let addr = ep.addr();
        if !addr.is_empty() {
            return addr;
        }
        time::sleep(Duration::from_millis(20)).await;
    }
    ep.addr()
}

pub async fn spawn_global_coordinator(
    base_path: &Path,
    r_url: &str,
    info_client: &Client,
) -> Result<(CoordinatorIroh, CoordinatorInfo, String)> {
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
            community_registry_url: Some(r_url.to_string()),
            share_in_registry: true,
            ..Default::default()
        }),
        ..Default::default()
    });
    let c = init_coordinator(&config_c).await?;
    wait_for_relay(&c, Duration::from_secs(30)).await?;
    let c_info_addr = c.info_addr().unwrap();

    let c_info: CoordinatorInfo =
        info_client.get(format!("http://{c_info_addr}{INFO_PATH}")).send().await?.json().await?;
    let c_relay_url = c_info.relay_url.clone().unwrap();

    Ok((c, c_info, c_relay_url))
}

pub async fn spawn_private_coordinator(
    base_path: &Path,
    r_url: &str,
    c_relay_url: &str,
    info_client: &Client,
) -> Result<(CoordinatorIroh, CoordinatorInfo)> {
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
            community_registry_url: Some(r_url.to_string()),
            share_in_registry: true,
            ..Default::default()
        }),
        ..Default::default()
    });
    config_cp.parent_coordinator.iroh.url = Some(c_relay_url.to_string());
    let cp = init_coordinator(&config_cp).await?;
    wait_for_relay(&cp, Duration::from_secs(30)).await?;
    let cp_info_addr = cp.info_addr().unwrap();

    let cp_info: CoordinatorInfo =
        info_client.get(format!("http://{cp_info_addr}{INFO_PATH}")).send().await?.json().await?;

    Ok((cp, cp_info))
}

pub struct InboundRelayTopology {
    pub registry_r: syneroym_community_registry::EcosystemRegistry,
    pub r_url: String,
    pub c: CoordinatorIroh,
    pub c_info: CoordinatorInfo,
    pub cp: CoordinatorIroh,
    pub target_z: TargetSubstrate,
}

impl InboundRelayTopology {
    pub async fn spawn(base_path: &Path, info_client: &Client) -> Result<Self> {
        let (registry_r, r_url) = spawn_registry(base_path).await?;
        let (c, c_info, c_relay_url) =
            spawn_global_coordinator(base_path, &r_url, info_client).await?;
        let (cp, cp_info) =
            spawn_private_coordinator(base_path, &r_url, &c_relay_url, info_client).await?;
        let target_z =
            spawn_z_target_substrate(base_path, &r_url, cp_info.relay_url.clone(), info_client)
                .await?;
        Ok(Self { registry_r, r_url, c, c_info, cp, target_z })
    }

    pub fn new_client(&self) -> SyneroymClient {
        SyneroymClient::new_with_mechanisms(
            self.target_z.did.clone(),
            vec![EndpointMechanism::Iroh {
                endpoint_addr_bytes: self.c_info.endpoint_addr_bytes.clone(),
                relay_url: self.c_info.relay_url.clone(),
            }],
        )
    }

    pub async fn shutdown(mut self) -> Result<()> {
        let _ = self.target_z.router.shutdown().await;
        self.cp.shutdown().await?;
        self.c.shutdown().await?;
        self.registry_r.shutdown().await?;
        Ok(())
    }
}

pub struct TargetSubstrate {
    pub did: String,
    pub router: Router,
    pub route_handler: RouteHandler,
}

pub async fn spawn_z_target_substrate(
    base_path: &Path,
    r_url: &str,
    cp_relay_url: Option<String>,
    info_client: &Client,
) -> Result<TargetSubstrate> {
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

    let mut ep_z_bldr = Endpoint::empty_builder();
    if let Some(relay_url) = cp_relay_url.as_ref().and_then(|r| r.parse::<RelayUrl>().ok()) {
        ep_z_bldr = ep_z_bldr.relay_mode(RelayMode::Custom(RelayMap::from(relay_url)));
    }
    let secret_key_z = SecretKey::from_bytes(&secret_z_bytes);
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
    let router_z = Router::builder(ep_z).accept(SYNEROYM_ALPN, route_handler_z.clone()).spawn();

    let signed_info_z = EndpointInfo {
        service_id: did_z.clone(),
        substrate_id: did_z.clone(),
        endpoint_type: EndpointType::Substrate,
        nickname: Some("test-node".to_string()),
        mechanisms: vec![EndpointMechanism::Iroh {
            endpoint_addr_bytes: serde_json::to_vec(&EndpointAddr::new(ep_z_addr.id)).unwrap(),
            relay_url: cp_relay_url,
        }],
        is_private: false,
        ttl: None,
        not_after: u64::MAX / 2,
        generation: 0,
        mailbox: None,
    }
    .sign(&identity_z)
    .unwrap();

    let res = info_client.post(format!("{r_url}/register")).json(&signed_info_z).send().await?;
    assert!(res.status().is_success());

    Ok(TargetSubstrate { did: did_z, router: router_z, route_handler: route_handler_z })
}

#[derive(Debug, Default)]
pub struct CapturingNativeService {
    pub captured_caller_did: Mutex<Option<String>>,
    pub captured_anchor_did: Mutex<Option<String>>,
    pub captured_on_behalf_did: Mutex<Option<String>>,
}

#[async_trait]
impl NativeService for CapturingNativeService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        *self.captured_caller_did.lock().unwrap() = Some(invocation.caller.caller_did.clone());
        *self.captured_anchor_did.lock().unwrap() = invocation.caller.session.anchor_did.clone();
        *self.captured_on_behalf_did.lock().unwrap() = invocation.caller.on_behalf_did.clone();
        Ok(NativeResponse { payload: serde_json::json!({"status": "ok"}) })
    }
}

pub struct NativeCapabilityCalleeNode {
    pub did: String,
    pub router: Router,
    pub capturing_service: Arc<CapturingNativeService>,
}

pub async fn spawn_native_capability_callee_node(
    base_path: &Path,
    r_url: &str,
) -> Result<NativeCapabilityCalleeNode> {
    let identity_z = Identity::generate()?;
    let secret_z_bytes = identity_z.to_bytes();
    let did_z = derive_did_key(&identity_z.public_key());

    let mut config_z = SubstrateConfig {
        app_local_data_dir: base_path.join("data_z"),
        app_data_dir: base_path.join("user_data_z"),
        ..Default::default()
    };
    config_z.substrate.enable_bep0044_dht = false;
    config_z.substrate.registry_url = Some(r_url.to_string());
    config_z.resolve_paths();
    let data_store_z = registry_store::init_store(&config_z).await?;
    let endpoint_registry_z = EndpointRegistry::new(data_store_z).await?;

    let ep_z =
        Endpoint::empty_builder().secret_key(SecretKey::from_bytes(&secret_z_bytes)).bind().await?;
    let ep_z_addr = wait_for_local_addr(&ep_z).await;

    let deps_z = build_test_route_handler_deps(&config_z, &did_z, &endpoint_registry_z).await?;
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

    Ok(NativeCapabilityCalleeNode { did: did_z, router: router_z, capturing_service })
}

pub struct NativeCapabilityCallerNode {
    pub did: String,
    pub ep: Endpoint,
    pub route_handler: RouteHandler,
    pub service_proxy: Arc<dyn ServiceProxy>,
}

pub async fn spawn_native_capability_caller_node(
    base_path: &Path,
    r_url: &str,
) -> Result<NativeCapabilityCallerNode> {
    let identity_x = Identity::generate()?;
    let secret_x_bytes = identity_x.to_bytes();
    let did_x = derive_did_key(&identity_x.public_key());

    let mut config_x = SubstrateConfig {
        app_local_data_dir: base_path.join("data_x"),
        app_data_dir: base_path.join("user_data_x"),
        ..Default::default()
    };
    config_x.substrate.enable_bep0044_dht = false;
    config_x.substrate.registry_url = Some(r_url.to_string());
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

    Ok(NativeCapabilityCallerNode {
        did: did_x,
        ep: ep_x,
        route_handler: route_handler_x,
        service_proxy,
    })
}

pub async fn build_delegated_proxy_request(
    r_url: &str,
    target_service_did: &str,
) -> Result<(String, ProxyRequest)> {
    let master = Identity::generate()?;
    let master_did = derive_did_key(&master.public_key());
    let temp = Identity::generate()?;
    let cert =
        DelegationCertificate::issue(&master, temp.public_key(), 3600, "routing".to_string())?;

    let registry_client = RegistryClient::new(false, Some(r_url.to_string()));
    registry_client.publish_master_anchor(&master_did, vec![], None, &master, false).await?;

    let mut preamble = RoutePreamble::new(
        RouteTransport::Binary,
        RouteProtocol::JsonRpc,
        target_service_did,
        "data-layer",
    );
    preamble.delegation = Some(cert);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    preamble.sign(&temp, now);
    let proof = preamble.caller_proof();

    let req = ProxyRequest {
        target_service: target_service_did.to_string(),
        interface: "data-layer".to_string(),
        method: "query".to_string(),
        params: serde_json::json!({}),
        caller: CallerContext {
            caller_did: master_did.clone(),
            app_instance: None,
            session: SessionContext::default(),
            auth: AuthLevel::Delegated,
            proof,
            on_behalf_did: None,
        },
        origin: CallOrigin::Native { service_id: None },
        protocol: ProxyProtocol::JsonRpcV1,
        idempotency_key: None,
        idempotent: false,
        timeout: Some(Duration::from_secs(5)),
    };

    Ok((master_did, req))
}
