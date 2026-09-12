//! Router setup, dependency construction, and composition root.

use std::sync::{Arc, Weak};

use dashmap::DashMap;
use syneroym_app_orchestration::{
    AppInstanceId, AppRegistry, LogicalResolver, LogicalServiceName, StaticInventory,
    TopologyEntry, TopologyKey,
};
use syneroym_control_plane::ControlPlaneService;
use syneroym_conversation::{
    ConversationConfig, ConversationService, store::ConversationConfig as StoreConversationConfig,
};
use syneroym_core::{
    asset_manifest::AssetRegistry,
    config::{BlobBackend, RetryPolicy, SubstrateConfig},
    dht_registry::RegistryClient,
    endpoint_publisher::EndpointPublisher,
    http_routes::HttpRouteRegistry,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    protocol_utils::{AUTH_SERVICE_ALIAS, SessionRevocationCheck},
    record_signer::NodeRecordSigner,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, registry_store, traits::StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_identity::{Identity, substrate::SubstrateIdentityStatus};
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_router::{ConnectionRouter, RouteHandlerDeps};
use syneroym_rpc::{
    ConversationHost, ConversationNotifier, NativeDispatchRegistry, NativeHttpRegistry,
    NativeHttpService, ServiceProxy, WebSocketSenders,
};
use syneroym_sandbox_podman::ContainerEngine;
use syneroym_sandbox_wasm::AppSandboxEngine;
use tokio::sync::mpsc;
use tracing::{debug, warn};

#[cfg(feature = "auth")]
use super::auth::init_auth_service;
#[cfg(feature = "dual_build_fixture")]
use super::dual_build_fixture::init_dual_build_fixture;
#[cfg(feature = "roym")]
use super::roym::init_roym;
use super::{
    handles::SharedNodeHandles,
    publish::publish_to_community_registry,
    supervisor::{SUPERVISOR_DISPATCH_ID, SupervisorHandle, init_supervisor},
};
use crate::identity;

/// Sets up the connection router and its tightly coupled dependencies,
/// including the substrate identity, data store, endpoint registry, and the
/// native service.
pub(super) async fn setup_connection_router(
    config: &SubstrateConfig,
) -> anyhow::Result<(
    ConnectionRouter,
    EndpointRegistry,
    Option<Arc<SupervisorHandle>>,
    Arc<ConversationService>,
    Option<String>,
)> {
    let (service_id, secret_key, verified_controller) = setup_identity_and_storage(config).await?;

    // A verified `ControllerAgreement` (mutually signed by the substrate and
    // its controller, see `identity::setup_substrate_identity`) is the
    // authoritative substrate owner and takes precedence over the plain
    // `[iam].admin_ucan_root` config string -- a controller cannot claim
    // ownership unilaterally, only a two-way-signed agreement counts. The
    // config value remains a fallback for deployments with no agreement
    // configured at all.
    let mut effective_config = config.clone();
    if let Some(controller) = verified_controller {
        effective_config.iam.admin_ucan_root = Some(controller);
    }
    let config = &effective_config;

    // No verified ControllerAgreement controller and no
    // [iam].admin_ucan_root means the substrate is unowned and fails
    // closed -- no caller holds any node-wide capability at all
    // (`build_caller`, `crates/router/src/route_handler/io.rs`). Logged
    // loudly because the operator-facing fix is a local, offline step.
    if config.iam.admin_ucan_root.is_none() {
        warn!(
            "substrate has no verified ControllerAgreement controller and no \
             [iam].admin_ucan_root: running UNOWNED and FAIL-CLOSED -- no caller can deploy, \
             undeploy, status-check, or reach the security interface (KEK/secrets) on this node. \
             Establish ownership on this host with: roymctl substrate claim --controller <name>  \
             (then restart)"
        );
    }

    let (router, endpoint_registry, _publisher, supervisor, conversation, auth_did) =
        setup_router(config, &service_id, secret_key).await?;

    Ok((router, endpoint_registry, supervisor, conversation, auth_did))
}

async fn setup_identity_and_storage(
    config: &SubstrateConfig,
) -> anyhow::Result<(String, [u8; 32], Option<String>)> {
    let substrate_identity_state =
        identity::setup_substrate_identity(&config.identity, &config.app_data_dir)?;
    let substrate_secret_key = identity::get_secret(&config.identity, &config.app_data_dir)?;
    // Only a *verified* (mutually signed) controller agreement establishes
    // substrate ownership -- `Unverified`/`None` never grant `substrate/admin`.
    let verified_controller = (substrate_identity_state.status
        == SubstrateIdentityStatus::Verified)
        .then_some(substrate_identity_state.controller)
        .flatten();
    Ok((substrate_identity_state.did, substrate_secret_key, verified_controller))
}

async fn setup_router(
    config: &SubstrateConfig,
    service_id: &str,
    secret_key: [u8; 32],
) -> anyhow::Result<(
    ConnectionRouter,
    EndpointRegistry,
    Option<Arc<EndpointPublisher>>,
    Option<Arc<SupervisorHandle>>,
    Arc<ConversationService>,
    Option<String>,
)> {
    let data_store = registry_store::init_store(config).await?;
    let endpoint_registry = EndpointRegistry::new(data_store).await?;

    debug!("Registering native SubstrateService at {}", service_id);
    let endpoint = SubstrateEndpoint::NativeHostChannel { service_id: service_id.to_string() };
    endpoint_registry
        .register(service_id.to_string(), "orchestrator".to_string(), endpoint)
        .await?;
    let security_endpoint =
        SubstrateEndpoint::NativeHostChannel { service_id: service_id.to_string() };
    endpoint_registry
        .register(service_id.to_string(), "security".to_string(), security_endpoint)
        .await?;

    let (route_handler_deps, shared) =
        build_route_handler_deps(config, service_id, &endpoint_registry, secret_key).await?;
    let control_plane = route_handler_deps.control_plane.clone();

    let supervisor = if config.roles.supervisor.is_some() {
        let supervisor_endpoint =
            SubstrateEndpoint::NativeHostChannel { service_id: SUPERVISOR_DISPATCH_ID.to_string() };
        endpoint_registry
            .register(service_id.to_string(), "supervisor".to_string(), supervisor_endpoint)
            .await?;
        // Gives the supervisor's own alert publication a `messaging`
        // endpoint to publish under -- a supervisor role is not a deployed
        // service, so without this registration nothing resolves
        // `SUPERVISOR_DISPATCH_ID` for the
        // `messaging` interface at all. **Deliberately** registered under
        // the same reserved id every other supervisor verb uses: the
        // router's own subscribe path (`dispatch.rs::handle_messaging_
        // subscribe`) namespaces this one service id with the
        // publish-side (unconditional-prefix) rule instead of the
        // ordinary subscribe-side rule every deployed service's
        // `messaging` endpoint gets -- see that function's own comment
        // for why. Do not "correct" that divergence back to the ordinary
        // rule: it is what keeps a caller's subscribe confined to
        // `svc/supervisor/...` on a node that hosts no deployed services
        // of its own to share the reach with.
        let messaging_endpoint =
            SubstrateEndpoint::NativeHostChannel { service_id: SUPERVISOR_DISPATCH_ID.to_string() };
        endpoint_registry
            .register(service_id.to_string(), "messaging".to_string(), messaging_endpoint)
            .await?;
        Some(init_supervisor(config, service_id, &shared).await?)
    } else {
        None
    };

    #[cfg(feature = "dual_build_fixture")]
    let fixture_factory = init_dual_build_fixture(&shared, &endpoint_registry, service_id).await?;
    // Only wire the Roym SynApp's native build when this node actually
    // runs the role. The `roym` feature can be compiled in (CI builds the
    // whole workspace with `--all-features`) without every substrate boot
    // wanting six extra `NativeService`s registered against no deploy
    // record.
    #[cfg(feature = "roym")]
    let roym_factories = if config.roles.roym.is_some() {
        init_roym(&shared, &endpoint_registry, service_id, config).await?
    } else {
        Vec::new()
    };

    let router = ConnectionRouter::init(
        endpoint_registry.clone(),
        config.clone(),
        secret_key,
        service_id.to_string(),
        route_handler_deps,
    )
    .await?;

    // `ProxyRouter` (the sole `ServiceProxy`) exists only now -- wired the
    // same post-construction way `AppSandboxEngine.service_proxy`/
    // `ControlPlaneService.service_proxy` already are.
    if let Some(proxy) = router.proxy() {
        ConversationService::set_service_proxy(
            shared.conversation(),
            Arc::downgrade(&proxy) as Weak<dyn ServiceProxy>,
        );
        #[cfg(feature = "dual_build_fixture")]
        if let Some(factory) = fixture_factory {
            factory.set_service_proxy(Arc::downgrade(&proxy) as Weak<dyn ServiceProxy>);
        }
        #[cfg(feature = "roym")]
        for factory in &roym_factories {
            factory.set_service_proxy(Arc::downgrade(&proxy) as Weak<dyn ServiceProxy>);
        }
    }

    // Built here rather than in `build_route_handler_deps` because it needs
    // the finished `EndpointRegistry`, and handed to the control plane so a
    // deploy can publish immediately instead of waiting for the heartbeat.
    let publisher = (config.substrate.registry_url.is_some()
        || config.substrate.enable_bep0044_dht)
        .then(|| {
            Arc::new(EndpointPublisher::new(
                Arc::new(RegistryClient::new(
                    config.substrate.enable_bep0044_dht,
                    config.substrate.registry_url.clone(),
                )),
                config.hosted_apps_dir(),
            ))
        });

    if let Some(publisher) = &publisher {
        // A registry is configured, so a deploy must be able to publish. A
        // type-erased control plane cannot, and silently skipping the wiring
        // would leave deploy-time publishing off with nothing to notice it.
        let control_plane = control_plane.ok_or_else(|| {
            anyhow::anyhow!(
                "a community registry is configured but no concrete ControlPlaneService was \
                 built, so a deploy could not publish its endpoint record"
            )
        })?;
        control_plane.set_endpoint_publisher(publisher.clone());

        // Wires `republish_now` to this heartbeat loop before spawning it,
        // so a caller can force an immediate republish (e.g. after a
        // community registry this node's records were wiped from comes
        // back up) instead of waiting up to `HEARTBEAT_INTERVAL_SECS`.
        if let Some(endpoint_addr) = router.endpoint_addr() {
            let relay_url = config.parent_coordinator.iroh.as_ref().map(|c| c.url.clone());
            let (force_tx, force_rx) = mpsc::channel(4);
            control_plane.set_republish_trigger(force_tx);
            publish_to_community_registry(
                service_id.to_string(),
                endpoint_addr,
                relay_url,
                secret_key,
                config.identity.nickname.clone(),
                publisher.clone(),
                force_rx,
            );
        }
    }

    let auth_did = shared
        .native_http()
        .get(AUTH_SERVICE_ALIAS)
        .and_then(|svc| NativeHttpService::service_id(&**svc.value()).map(ToString::to_string));

    Ok((router, endpoint_registry, publisher, supervisor, shared.conversation().clone(), auth_did))
}

/// Rebuilds the in-memory `StaticInventory` from every dependency binding
/// `EndpointRegistry` has persisted (ADR-0021 §5) -- a restarted
/// substrate must answer a guest's first call, and nothing re-pushes on
/// restart. A row that fails to parse is warned and skipped, exactly like
/// the unparseable-`TopologyEntry`-JSON case beside it: every one of the
/// three stored strings is caller-supplied at some point in its history,
/// so `LogicalServiceName::new` would *panic* substrate startup on a row
/// containing a `/`, which is a strictly worse outcome than skipping that
/// one row.
pub(super) async fn replay_persisted_bindings(
    registry: &EndpointRegistry,
) -> anyhow::Result<Arc<StaticInventory>> {
    let app_registry = Arc::new(StaticInventory::new());
    for (_service_id, instance, dep_name, entry_json) in registry.all_bindings().await? {
        let parsed = (|| -> anyhow::Result<_> {
            Ok((
                AppInstanceId::try_new(&instance)?,
                LogicalServiceName::try_new(&dep_name)?,
                serde_json::from_str::<TopologyEntry>(&entry_json)?,
            ))
        })();
        match parsed {
            Ok((instance_id, service_name, entry)) => {
                app_registry.register(TopologyKey::local(instance_id, service_name), entry);
            }
            Err(e) => {
                warn!(%instance, %dep_name, error = %e, "skipping an unreadable persisted binding");
            }
        }
    }
    Ok(app_registry)
}

/// Constructs every capability the connection router holds and dispatches
/// through but does not itself build: storage, blob, and messaging
/// backends, the WASM and container sandboxes, and the control-plane
/// native service. This is the substrate's composition root -- `router`
/// only needs the finished handles, not the knowledge of how to build them.
async fn build_route_handler_deps(
    config: &SubstrateConfig,
    service_id: &str,
    registry: &EndpointRegistry,
    secret_key: [u8; 32],
) -> anyhow::Result<(RouteHandlerDeps, SharedNodeHandles)> {
    // Shared with `ControlPlaneService`'s native `data-layer` dispatch
    // (`SynSvcNativeService`), which signs relationship-proof
    // records as this node's own asserter identity -- the same key material
    // `ConnectionRouter::init` (below, in the caller) separately constructs
    // its own `Identity` from for `ProxyRouter`'s `node_identity`.
    let node_identity = Arc::new(Identity::from_bytes(&secret_key));
    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(&config.storage.db_dir, config.storage.encryption)?);
    let blob_provider = build_blob_provider(config)?;

    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig {
        channel_capacity: config.mqtt.channel_capacity as usize,
    })?);

    // ADR-0021 §2: replay persisted bindings before anything can
    // resolve one -- a restarted substrate must answer a guest's first
    // call, and nothing re-pushes on restart (ADR-0021 §5 -- push failure
    // is sticky, and so is push absence).
    let app_registry = replay_persisted_bindings(registry).await?;
    let logical_resolver = Arc::new(LogicalResolver::new(app_registry));

    let (app_sandbox_engine, podman_sandbox_engine, websocket_senders) = build_sandbox_engines(
        config,
        registry,
        &key_store,
        &storage_provider,
        &blob_provider,
        &messaging_broker,
        &logical_resolver,
    )
    .await?;

    // Shared with `ControlPlaneService`, which registers/deregisters
    // per-deployment native services (data-layer/vault/app-config/
    // blob-store) and HTTP routes into these same tables on deploy/undeploy
    // -- `RouteHandler`'s own dispatch path reads through the identical
    // handles.
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let native_http: NativeHttpRegistry = Arc::new(DashMap::new());
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let assets: AssetRegistry = Arc::new(DashMap::new());

    // Cloning the `Arc`, not `node_identity` itself -- `Identity`
    // deliberately does not implement `Clone`, but a second handle to the
    // same key material is exactly what's needed here. The supervisor role
    // (when configured) uses this as the identity it presents when it
    // connects, as a client, to the substrates it manages (ADR-0021 §8).
    let supervisor_client_identity = node_identity.clone();

    let conversation = build_conversation_service(config, &storage_provider, &key_store, registry)?;

    // Both directions, `Weak` on both sides: the engine reaches
    // `conversation` for the guest-facing WIT surface, `conversation`
    // reaches the engine to notify a wasm-hosted service of an inbound
    // message/state change.
    app_sandbox_engine
        .conversation
        .set(Arc::downgrade(&conversation) as Weak<dyn ConversationHost>)
        .map_err(|_| anyhow::anyhow!("AppSandboxEngine::conversation set more than once"))?;
    conversation
        .set_notifier(Arc::downgrade(&app_sandbox_engine) as Weak<dyn ConversationNotifier>);

    let record_signer = NodeRecordSigner::new(node_identity.clone(), registry.clone());
    let _ = app_sandbox_engine.record_signer.set(record_signer.clone());

    let control_plane_service = ControlPlaneService::init(
        service_id.to_string(),
        service_id.to_string(),
        app_sandbox_engine.clone(),
        podman_sandbox_engine,
        registry.clone(),
        config.hosted_apps_dir(),
        key_store.clone(),
        storage_provider.clone(),
        blob_provider.clone(),
        messaging_broker.clone(),
        native_dispatch.clone(),
        http_routes.clone(),
        assets.clone(),
        node_identity,
        logical_resolver.clone(),
    )
    .await?;
    let control_plane_service = Arc::new(control_plane_service);
    control_plane_service.set_record_signer(record_signer.clone());
    // Unlike `service_proxy`/`row_authorizer` (which need `ProxyRouter`,
    // built later in `ConnectionRouter::init`), `ConversationService`
    // already exists at this point, so it is wired in here rather than
    // deferred to `setup_router`.
    control_plane_service
        .conversation
        .set(Arc::downgrade(&conversation) as Weak<dyn ConversationHost>)
        .map_err(|_| anyhow::anyhow!("ControlPlaneService::conversation set more than once"))?;

    let shared = SharedNodeHandles::new(
        key_store.clone(),
        storage_provider.clone(),
        native_dispatch.clone(),
        supervisor_client_identity,
        messaging_broker.clone(),
        blob_provider,
        logical_resolver.clone(),
        conversation.clone(),
        http_routes.clone(),
        native_http.clone(),
        websocket_senders.clone(),
        assets.clone(),
        record_signer,
    );

    #[cfg(feature = "auth")]
    let auth_service = init_auth_service(config, &shared, registry, service_id).await?;
    #[cfg(not(feature = "auth"))]
    let auth_service: Option<Arc<dyn SessionRevocationCheck>> = None;

    Ok((
        RouteHandlerDeps {
            logical_resolver: logical_resolver.clone(),
            key_store,
            storage_provider,
            app_sandbox_engine,
            messaging_broker,
            native_dispatch,
            native_http,
            websocket_senders,
            http_routes,
            assets,
            sse_permits: control_plane_service.sse_permits(),
            control_plane_service: control_plane_service.clone(),
            control_plane: Some(control_plane_service),
            session_revocation: auth_service.map(|a| a as Arc<dyn SessionRevocationCheck>),
        },
        shared,
    ))
}

/// Builds and wires the WASM app sandbox engine, the container sandbox
/// engine, and the shared WebSocket sender table. Extracted from
/// `build_route_handler_deps` so that function reads as sequential
/// composition rather than one large allocation block.
async fn build_sandbox_engines(
    config: &SubstrateConfig,
    registry: &EndpointRegistry,
    key_store: &Arc<KeyStore>,
    storage_provider: &Arc<dyn StorageProvider>,
    blob_provider: &Arc<dyn BlobProvider>,
    messaging_broker: &Arc<MqttBroker>,
    logical_resolver: &Arc<LogicalResolver>,
) -> anyhow::Result<(Arc<AppSandboxEngine>, Arc<ContainerEngine>, Arc<WebSocketSenders>)> {
    let app_sandbox_engine = Arc::new(
        AppSandboxEngine::init(
            config,
            registry.get_all_endpoints(),
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            registry.clone(),
            logical_resolver.clone(),
        )
        .await?,
    );
    app_sandbox_engine
        .self_weak
        .set(Arc::downgrade(&app_sandbox_engine))
        .map_err(|_| anyhow::anyhow!("AppSandboxEngine::self_weak set more than once"))?;
    let websocket_senders = WebSocketSenders::new();
    app_sandbox_engine
        .websocket_senders
        .set(websocket_senders.clone())
        .map_err(|_| anyhow::anyhow!("AppSandboxEngine::websocket_senders set more than once"))?;

    replay_persisted_subscriptions(storage_provider, &app_sandbox_engine).await?;

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

    Ok((app_sandbox_engine, podman_sandbox_engine, websocket_senders))
}

/// Constructs the `ConversationService` with its full config derived from
/// the substrate config's `app_sandbox` role. Extracted from
/// `build_route_handler_deps`; the caller is responsible for wiring the
/// bidirectional `Weak` links to `AppSandboxEngine` and `ControlPlaneService`
/// afterwards (those components don't exist at construction time).
fn build_conversation_service(
    config: &SubstrateConfig,
    storage_provider: &Arc<dyn StorageProvider>,
    key_store: &Arc<KeyStore>,
    registry: &EndpointRegistry,
) -> anyhow::Result<Arc<ConversationService>> {
    // Same lifetime as `app_sandbox_engine` above -- built
    // once, wired to the real `ServiceProxy`/engine notifier once those
    // exist (`setup_router`, after `ConnectionRouter::init`).
    let app_sandbox_role = config.roles.app_sandbox.clone().unwrap_or_default();
    ConversationService::new(
        storage_provider.clone(),
        key_store.clone(),
        registry.clone(),
        syneroym_async_queue::QueueConfig {
            retry: RetryPolicy {
                max_attempts: 54,
                initial_backoff_ms: 100,
                backoff_multiplier: 2.0,
                max_backoff_ms: 900_000,
            },
            visibility_timeout_ms: 120_000,
            dlq_max_rows: 1000,
            max_pending_rows: syneroym_async_queue::DEFAULT_MAX_PENDING_ROWS,
        },
        ConversationConfig {
            store: StoreConversationConfig {
                max_body_bytes: app_sandbox_role.conversation_max_body_bytes,
                max_pending_per_conversation: app_sandbox_role
                    .conversation_max_pending_per_conversation,
                max_messages_per_conversation: app_sandbox_role
                    .conversation_max_messages_per_conversation,
                max_pending_age_secs: app_sandbox_role.conversation_max_pending_age_secs,
                max_clock_skew_secs: app_sandbox_role.conversation_max_clock_skew_secs,
                prekey_requests_per_peer_per_hour: app_sandbox_role
                    .conversation_prekey_requests_per_peer_per_hour,
                conversation_group_sync_secs: app_sandbox_role.conversation_group_sync_secs,
                conversation_group_rekey_secs: app_sandbox_role.conversation_group_rekey_secs,
                conversation_max_group_members: app_sandbox_role.conversation_max_group_members,
                conversation_max_dag_entries_per_conversation: app_sandbox_role
                    .conversation_max_dag_entries_per_conversation,
                conversation_max_sync_entries_per_call: app_sandbox_role
                    .conversation_max_sync_entries_per_call,
                conversation_relay_fanout: app_sandbox_role.conversation_relay_fanout,
                conversation_sync_now_budget_ms: app_sandbox_role.conversation_sync_now_budget_ms,
                conversation_background_sync_budget_ms: app_sandbox_role
                    .conversation_background_sync_budget_ms,
            },
        },
    )
}

/// Guest subscriptions survive a restart (ADR-0010 Finding A1): replay
/// every persisted row into the broker before the router starts accepting
/// connections. Best-effort per row -- one bad topic shouldn't block
/// substrate startup. Replayed concurrently (independent rows, no shared
/// state) to keep this bounded by the slowest single subscribe rather than
/// their sum.
async fn replay_persisted_subscriptions(
    storage_provider: &Arc<dyn StorageProvider>,
    app_sandbox_engine: &AppSandboxEngine,
) -> anyhow::Result<()> {
    let persisted_subscriptions = storage_provider.list_all_messaging_subscriptions().await?;
    let replay_results = futures::future::join_all(persisted_subscriptions.iter().map(
        |(subscribed_service_id, topic)| {
            app_sandbox_engine.register_internal_subscription(subscribed_service_id, topic)
        },
    ))
    .await;
    for ((subscribed_service_id, topic), result) in
        persisted_subscriptions.iter().zip(replay_results)
    {
        if let Err(e) = result {
            warn!(
                service_id = %subscribed_service_id,
                topic = %topic,
                error = %e,
                "Failed to replay messaging subscription on startup"
            );
        }
    }
    Ok(())
}

/// Constructs the configured blob backend (`Local` or `S3`). `S3` requires
/// building with the `aws` cargo feature (off by default -- see the
/// `object_store`/`digest` version-pin comment in the root `Cargo.toml`);
/// selecting it otherwise fails fast here with an actionable message rather
/// than silently falling back to `Local`.
fn build_blob_provider(config: &SubstrateConfig) -> anyhow::Result<Arc<dyn BlobProvider>> {
    let bs = &config.storage.blob_store;
    match bs.backend {
        BlobBackend::Local => Ok(Arc::new(ObjectStoreBlobProvider::new_local(
            bs.local_root.clone(),
            bs.max_blob_bytes,
            bs.max_service_total_bytes,
        )?)),
        BlobBackend::S3 => {
            #[cfg(feature = "aws")]
            {
                let s3 = bs.s3.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "storage.blob_store.backend = \"s3\" requires [storage.blob_store.s3] to \
                         be configured"
                    )
                })?;
                Ok(Arc::new(ObjectStoreBlobProvider::new_s3(
                    &s3.endpoint,
                    &s3.bucket,
                    &s3.region,
                    bs.max_blob_bytes,
                    bs.max_service_total_bytes,
                )?))
            }
            #[cfg(not(feature = "aws"))]
            {
                Err(anyhow::anyhow!(
                    "storage.blob_store.backend = \"s3\" requires building syneroym-substrate \
                     with the `aws` feature (off by default -- see the object_store/digest \
                     version-pin comment in the root Cargo.toml)"
                ))
            }
        }
    }
}
