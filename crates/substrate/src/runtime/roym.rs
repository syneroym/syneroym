//! Roym native SynApp initialization and dispatch.

#[cfg(feature = "roym")]
use std::sync::Arc;

#[cfg(feature = "roym")]
use syneroym_app_host_native::NativeHostFactory;
#[cfg(feature = "roym")]
use syneroym_core::{
    config::SubstrateConfig,
    http_routes::HttpRoute,
    local_registry::{EndpointRegistry, HTTP_NATIVE_INTERFACE},
};

#[cfg(feature = "roym")]
use super::handles::SharedNodeHandles;

#[cfg(feature = "roym")]
const ROYM_APP_INSTANCE: &str = "roym";

#[cfg(feature = "roym")]
/// Internal dispatch id for one Roym service's native build — the key its
/// `NativeService`, its `endpoint_registry` `NativeHostChannel`, and every
/// sibling `TopologyEntry` member share. Shaped as a `did:key:` string so
/// it satisfies `ServiceId`'s invariant; it is never resolved as a real
/// DID (native dispatch is in-process, no handshake), only matched.
///
/// An inbound remote stream naming this id fails the E2E handshake because
/// no ed25519 private key exists for `did:key:roym-*`, and native services
/// are registered only in the local registry (`EndpointRegistry::register`),
/// never published to the external community registry.
fn roym_dispatch_id(name: &str) -> String {
    format!("did:key:roym-{name}")
}

#[cfg(feature = "roym")]
fn roym_http_routes() -> Vec<HttpRoute> {
    vec![
        HttpRoute {
            method: "POST".into(),
            path: "/rpc".into(),
            target: "guest".into(),
            operation: "handle-request".into(),
            collection: None,
            topic: None,
            protocol: None,
            public: true,
        },
        HttpRoute {
            method: "GET".into(),
            path: "/health".into(),
            target: "guest".into(),
            operation: "handle-request".into(),
            collection: None,
            topic: None,
            protocol: None,
            public: true,
        },
        HttpRoute {
            method: "GET".into(),
            path: "/ws".into(),
            target: "websocket".into(),
            operation: "handle-upgrade".into(),
            collection: None,
            topic: None,
            protocol: None,
            public: true,
        },
    ]
}

/// Initialises all six Roym services, wires HTTP and topology, and optionally
/// loads the Hub UI bundle. Sequencing function — each phase is extracted
/// into a named helper below.
#[cfg(feature = "roym")]
pub(super) async fn init_roym(
    shared: &SharedNodeHandles,
    endpoint_registry: &EndpointRegistry,
    node_service_id: &str,
    config: &SubstrateConfig,
) -> anyhow::Result<Vec<Arc<NativeHostFactory>>> {
    use syneroym_identity::substrate;
    use syneroym_roym_core::services;

    // 1. One factory + one NativeService per service (web includes HTTP surface).
    let factory_web = init_roym_web(shared, endpoint_registry, node_service_id).await?;
    let factory_profile = init_roym_profile(shared, endpoint_registry).await?;
    let factory_conv = init_roym_conversation(shared, endpoint_registry).await?;
    let factory_cat = init_roym_catalog(shared, endpoint_registry).await?;
    let factory_tx = init_roym_transaction(shared, endpoint_registry).await?;
    let factory_dir = init_roym_directory(shared, endpoint_registry).await?;

    let factories =
        vec![factory_web, factory_profile, factory_conv, factory_cat, factory_tx, factory_dir];

    // 2. App context and dependency bindings.
    let web_id = roym_dispatch_id(services::WEB.name);
    wire_roym_topology(shared, endpoint_registry, &web_id).await?;

    // 3. The UI bundle.
    load_roym_ui_bundle(shared, &web_id, config).await;

    for factory in &factories {
        factory.set_record_signer(shared.record_signer().clone());
    }

    if let Some(owner_did) = config.roles.roym.as_ref().and_then(|r| r.owner_did.as_ref()) {
        substrate::resolve_did_key(owner_did)
            .map_err(|e| anyhow::anyhow!("invalid roym.owner_did '{owner_did}': {e}"))?;
        for name in &[
            services::WEB.name,
            services::PROFILE.name,
            services::CONVERSATION.name,
            services::CATALOG.name,
            services::TRANSACTION.name,
            services::DIRECTORY.name,
        ] {
            endpoint_registry.set_owner(roym_dispatch_id(name), owner_did.clone()).await?;
        }
    }

    Ok(factories)
}

// ── Per-service init helpers ─────────────────────────────────────────────────

/// Builds the Roym `web` service factory, native instance, and wires its
/// inbound HTTP/WebSocket adapter surfaces into `native_http`, `http_routes`,
/// and `endpoint_registry`. `web` alone among the six Roym services exposes an
/// HTTP surface.
#[cfg(feature = "roym")]
async fn init_roym_web(
    shared: &SharedNodeHandles,
    endpoint_registry: &EndpointRegistry,
    node_service_id: &str,
) -> anyhow::Result<Arc<NativeHostFactory>> {
    use std::sync::Weak;

    use syneroym_app_host_native::{HttpSink, NativeHttpAdapter, WebSocketSink};
    use syneroym_core::local_registry::SubstrateEndpoint;
    use syneroym_roym_core::services;
    use syneroym_rpc::{NativeHttpService, NativeService};

    let web_id = roym_dispatch_id(services::WEB.name);
    let factory = NativeHostFactory::new(
        web_id.clone(),
        shared.key_store().clone(),
        shared.storage_provider().clone(),
        shared.blob_provider().clone(),
        shared.messaging_broker().clone(),
        endpoint_registry.clone(),
        shared.logical_resolver().clone(),
        shared.conversation().clone(),
        shared.websocket_senders().clone(),
    );
    let f = factory.clone();
    let f_http = factory.clone();
    let web = Arc::new(syneroym_roym_web::native::NativeWeb::new(
        web_id.clone(),
        move |caller| f.host_for(caller),
        // A guest HTTP / websocket request is router ingress, so the
        // native shim builds a wire-origin host for it, matching the WASM
        // engine's unconditional `from_wire` on every guest HTTP request.
        move |caller| f_http.host_for_wire(caller),
    ));
    shared.native_dispatch().insert(web_id.clone(), web.clone() as Arc<dyn NativeService>);
    endpoint_registry
        .register(
            web_id.clone(),
            services::WEB.interface.to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: web_id.clone() },
        )
        .await?;

    // Wire HTTP surface for `web`
    factory.set_http_sink(Arc::downgrade(&web) as Weak<dyn HttpSink>);
    factory.set_websocket_sink(Arc::downgrade(&web) as Weak<dyn WebSocketSink>);
    let adapter = Arc::new(NativeHttpAdapter::new(
        factory.clone(),
        Arc::downgrade(&web) as Weak<dyn HttpSink>,
        Arc::downgrade(&web) as Weak<dyn WebSocketSink>,
    ));
    shared.native_http().insert(web_id.clone(), adapter.clone() as Arc<dyn NativeHttpService>);
    shared.native_http().insert(node_service_id.to_string(), adapter as Arc<dyn NativeHttpService>);
    shared.http_routes().insert(web_id.clone(), roym_http_routes());
    shared.http_routes().insert(node_service_id.to_string(), roym_http_routes());
    endpoint_registry
        .register(
            web_id.clone(),
            HTTP_NATIVE_INTERFACE.to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: web_id.clone() },
        )
        .await?;
    endpoint_registry
        .register(
            node_service_id.to_string(),
            HTTP_NATIVE_INTERFACE.to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: web_id },
        )
        .await?;

    Ok(factory)
}

#[cfg(feature = "roym")]
async fn init_roym_profile(
    shared: &SharedNodeHandles,
    endpoint_registry: &EndpointRegistry,
) -> anyhow::Result<Arc<NativeHostFactory>> {
    use syneroym_core::local_registry::SubstrateEndpoint;
    use syneroym_roym_core::services;
    use syneroym_rpc::NativeService;

    let factory = NativeHostFactory::new(
        roym_dispatch_id(services::PROFILE.name),
        shared.key_store().clone(),
        shared.storage_provider().clone(),
        shared.blob_provider().clone(),
        shared.messaging_broker().clone(),
        endpoint_registry.clone(),
        shared.logical_resolver().clone(),
        shared.conversation().clone(),
        shared.websocket_senders().clone(),
    );
    let f = factory.clone();
    let profile = Arc::new(syneroym_roym_profile::native::NativeProfile::new(
        roym_dispatch_id(services::PROFILE.name),
        move |caller| f.host_for(caller),
    ));
    shared
        .native_dispatch()
        .insert(roym_dispatch_id(services::PROFILE.name), profile as Arc<dyn NativeService>);
    endpoint_registry
        .register(
            roym_dispatch_id(services::PROFILE.name),
            services::PROFILE.interface.to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: roym_dispatch_id(services::PROFILE.name),
            },
        )
        .await?;
    Ok(factory)
}

#[cfg(feature = "roym")]
async fn init_roym_conversation(
    shared: &SharedNodeHandles,
    endpoint_registry: &EndpointRegistry,
) -> anyhow::Result<Arc<NativeHostFactory>> {
    use std::sync::Weak;

    use syneroym_app_host_native::ConversationSink;
    use syneroym_core::local_registry::SubstrateEndpoint;
    use syneroym_roym_core::services;
    use syneroym_rpc::NativeService;

    let factory = NativeHostFactory::new(
        roym_dispatch_id(services::CONVERSATION.name),
        shared.key_store().clone(),
        shared.storage_provider().clone(),
        shared.blob_provider().clone(),
        shared.messaging_broker().clone(),
        endpoint_registry.clone(),
        shared.logical_resolver().clone(),
        shared.conversation().clone(),
        shared.websocket_senders().clone(),
    );
    let f = factory.clone();
    let conv = Arc::new(syneroym_roym_conversation::native::NativeConversation::new(
        roym_dispatch_id(services::CONVERSATION.name),
        move |caller| f.host_for(caller),
    ));
    // The delivery worker wakes the natively linked inbox the same way
    // `AppSandboxEngine` wakes a wasm-hosted one — without this an
    // inbound message reaches the host store and stops there.
    factory.set_conversation_sink(Arc::downgrade(&conv) as Weak<dyn ConversationSink>);
    shared
        .native_dispatch()
        .insert(roym_dispatch_id(services::CONVERSATION.name), conv as Arc<dyn NativeService>);
    endpoint_registry
        .register(
            roym_dispatch_id(services::CONVERSATION.name),
            services::CONVERSATION.interface.to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: roym_dispatch_id(services::CONVERSATION.name),
            },
        )
        .await?;
    Ok(factory)
}

#[cfg(feature = "roym")]
async fn init_roym_catalog(
    shared: &SharedNodeHandles,
    endpoint_registry: &EndpointRegistry,
) -> anyhow::Result<Arc<NativeHostFactory>> {
    use syneroym_core::local_registry::SubstrateEndpoint;
    use syneroym_roym_core::services;
    use syneroym_rpc::NativeService;

    let factory = NativeHostFactory::new(
        roym_dispatch_id(services::CATALOG.name),
        shared.key_store().clone(),
        shared.storage_provider().clone(),
        shared.blob_provider().clone(),
        shared.messaging_broker().clone(),
        endpoint_registry.clone(),
        shared.logical_resolver().clone(),
        shared.conversation().clone(),
        shared.websocket_senders().clone(),
    );
    let f = factory.clone();
    let cat = Arc::new(syneroym_roym_catalog::native::NativeCatalog::new(
        roym_dispatch_id(services::CATALOG.name),
        move |caller| f.host_for(caller),
    ));
    shared
        .native_dispatch()
        .insert(roym_dispatch_id(services::CATALOG.name), cat as Arc<dyn NativeService>);
    endpoint_registry
        .register(
            roym_dispatch_id(services::CATALOG.name),
            services::CATALOG.interface.to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: roym_dispatch_id(services::CATALOG.name),
            },
        )
        .await?;
    Ok(factory)
}

#[cfg(feature = "roym")]
async fn init_roym_transaction(
    shared: &SharedNodeHandles,
    endpoint_registry: &EndpointRegistry,
) -> anyhow::Result<Arc<NativeHostFactory>> {
    use syneroym_core::local_registry::SubstrateEndpoint;
    use syneroym_roym_core::services;
    use syneroym_rpc::NativeService;

    let factory = NativeHostFactory::new(
        roym_dispatch_id(services::TRANSACTION.name),
        shared.key_store().clone(),
        shared.storage_provider().clone(),
        shared.blob_provider().clone(),
        shared.messaging_broker().clone(),
        endpoint_registry.clone(),
        shared.logical_resolver().clone(),
        shared.conversation().clone(),
        shared.websocket_senders().clone(),
    );
    let f = factory.clone();
    let tx = Arc::new(syneroym_roym_transaction::native::NativeTransaction::new(
        roym_dispatch_id(services::TRANSACTION.name),
        move |caller| f.host_for(caller),
    ));
    shared
        .native_dispatch()
        .insert(roym_dispatch_id(services::TRANSACTION.name), tx as Arc<dyn NativeService>);
    endpoint_registry
        .register(
            roym_dispatch_id(services::TRANSACTION.name),
            services::TRANSACTION.interface.to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: roym_dispatch_id(services::TRANSACTION.name),
            },
        )
        .await?;
    Ok(factory)
}

#[cfg(feature = "roym")]
async fn init_roym_directory(
    shared: &SharedNodeHandles,
    endpoint_registry: &EndpointRegistry,
) -> anyhow::Result<Arc<NativeHostFactory>> {
    use syneroym_core::local_registry::SubstrateEndpoint;
    use syneroym_roym_core::services;
    use syneroym_rpc::NativeService;

    let factory = NativeHostFactory::new(
        roym_dispatch_id(services::DIRECTORY.name),
        shared.key_store().clone(),
        shared.storage_provider().clone(),
        shared.blob_provider().clone(),
        shared.messaging_broker().clone(),
        endpoint_registry.clone(),
        shared.logical_resolver().clone(),
        shared.conversation().clone(),
        shared.websocket_senders().clone(),
    );
    let f = factory.clone();
    let dir = Arc::new(syneroym_roym_directory::native::NativeDirectory::new(
        roym_dispatch_id(services::DIRECTORY.name),
        move |caller| f.host_for(caller),
    ));
    shared
        .native_dispatch()
        .insert(roym_dispatch_id(services::DIRECTORY.name), dir as Arc<dyn NativeService>);
    endpoint_registry
        .register(
            roym_dispatch_id(services::DIRECTORY.name),
            services::DIRECTORY.interface.to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: roym_dispatch_id(services::DIRECTORY.name),
            },
        )
        .await?;
    Ok(factory)
}

// ── Topology and UI bundle wiring ────────────────────────────────────────────

/// Registers the Roym app context for every service and persists all
/// intra-app dependency bindings so they survive a restart.
#[cfg(feature = "roym")]
async fn wire_roym_topology(
    shared: &SharedNodeHandles,
    endpoint_registry: &EndpointRegistry,
    web_id: &str,
) -> anyhow::Result<()> {
    use std::time::Duration;

    use syneroym_app_orchestration::{
        AppInstanceId, LogicalServiceName, TopologyEntry, TopologyEpoch, TopologyKey, TopologyMode,
        models::ServiceId,
    };
    use syneroym_roym_core::services;

    for svc in services::ALL {
        endpoint_registry
            .set_app_context(
                roym_dispatch_id(svc.name),
                ROYM_APP_INSTANCE.to_string(),
                svc.name.to_string(),
            )
            .await?;
    }
    for dep in services::SIBLINGS {
        let entry = TopologyEntry {
            mode: TopologyMode::Singleton,
            members: vec![ServiceId::new(roym_dispatch_id(dep.name))],
            sharding_strategy: None,
            epoch: TopologyEpoch(1),
            cache_ttl: Duration::from_secs(60),
            not_after: None,
        };
        shared.logical_resolver().register(
            TopologyKey::local(
                AppInstanceId::new(ROYM_APP_INSTANCE),
                LogicalServiceName::new(dep.name),
            ),
            entry.clone(),
        );
        endpoint_registry
            .save_binding(web_id, ROYM_APP_INSTANCE, dep.name, &serde_json::to_string(&entry)?)
            .await?;
    }

    // `conversation` and `catalog` each declare a `profile` dependency in
    // the manifest; resolution already works without the binding (`web`
    // declares it), but the native build's persisted bindings should
    // match the manifest all the same.
    let profile_entry = TopologyEntry {
        mode: TopologyMode::Singleton,
        members: vec![ServiceId::new(roym_dispatch_id(services::PROFILE.name))],
        sharding_strategy: None,
        epoch: TopologyEpoch(1),
        cache_ttl: Duration::from_secs(60),
        not_after: None,
    };
    let profile_entry_json = serde_json::to_string(&profile_entry)?;
    for consumer in [services::CONVERSATION.name, services::CATALOG.name] {
        endpoint_registry
            .save_binding(
                &roym_dispatch_id(consumer),
                ROYM_APP_INSTANCE,
                services::PROFILE.name,
                &profile_entry_json,
            )
            .await?;
    }

    // `directory` declares a `catalog` dependency: a provider's own
    // `directory.publish-to-source` reads the signed envelope from
    // `catalog` through this edge before sending it to a chosen source.
    let catalog_entry = TopologyEntry {
        mode: TopologyMode::Singleton,
        members: vec![ServiceId::new(roym_dispatch_id(services::CATALOG.name))],
        sharding_strategy: None,
        epoch: TopologyEpoch(1),
        cache_ttl: Duration::from_secs(60),
        not_after: None,
    };
    endpoint_registry
        .save_binding(
            &roym_dispatch_id(services::DIRECTORY.name),
            ROYM_APP_INSTANCE,
            services::CATALOG.name,
            &serde_json::to_string(&catalog_entry)?,
        )
        .await?;

    Ok(())
}

/// Unpacks the Hub UI bundle from the configured path and registers it in
/// the blob store and asset table. Errors are warned and skipped rather
/// than propagated — a missing or unreadable bundle degrades to serving
/// the API without the Hub, not a startup failure.
#[cfg(feature = "roym")]
async fn load_roym_ui_bundle(shared: &SharedNodeHandles, web_id: &str, config: &SubstrateConfig) {
    use std::{collections::BTreeSet, fs};

    use syneroym_control_plane::assets;
    use syneroym_core::asset_manifest::ServiceAssets;

    let Some(path) = config.roles.roym.as_ref().and_then(|r| r.ui_bundle_path.as_ref()) else {
        tracing::info!("no roym.ui_bundle_path configured; serving the API without the Hub");
        return;
    };

    let archive = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(?path, %e, "Roym UI bundle could not be read; serving API without Hub");
            return;
        }
    };

    // A DEK load failure must not silently downgrade to unpacking the
    // bundle unencrypted — skip bundle registration and log a warning
    // instead, the same way an unpack or manifest-store failure below does.
    let dek = match shared.storage_provider().load_service_dek(web_id, shared.key_store()).await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(
                ?path,
                %e,
                "Roym UI bundle's service DEK could not be loaded; serving API \
                 without Hub rather than unpacking it unencrypted"
            );
            return;
        }
    };

    let mut written = BTreeSet::new();
    let manifest = assets::unpack_asset_bundle(
        web_id,
        &archive,
        None,
        &roym_http_routes(),
        shared.blob_provider(),
        dek.clone(),
        &mut written,
    )
    .await;

    match manifest {
        Ok(m) => match assets::store_manifest(web_id, &m, shared.blob_provider(), dek).await {
            Ok(manifest_hash) => {
                shared.assets().insert(
                    web_id.to_string(),
                    ServiceAssets { manifest: Arc::new(m), public: true, manifest_hash },
                );
            }
            Err(e) => {
                tracing::warn!(
                    ?path,
                    %e,
                    "Roym UI bundle manifest could not be stored; serving API without Hub"
                );
            }
        },
        Err(e) => {
            tracing::warn!(?path, %e, "Roym UI bundle unpack failed; serving API without Hub");
        }
    }
}
