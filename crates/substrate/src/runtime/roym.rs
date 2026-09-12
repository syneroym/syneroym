//! Roym native SynApp initialization and dispatch.

#[cfg(feature = "roym")]
use std::sync::Arc;

#[cfg(feature = "roym")]
use syneroym_core::{config::SubstrateConfig, local_registry::EndpointRegistry};

#[cfg(feature = "roym")]
use super::router::SharedNodeHandles;

#[cfg(feature = "roym")]
const ROYM_APP_INSTANCE: &str = "roym";

#[cfg(feature = "roym")]
/// Internal dispatch id for one Roym service's native build -- the key its
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
fn roym_http_routes() -> Vec<syneroym_core::http_routes::HttpRoute> {
    use syneroym_core::http_routes::HttpRoute;
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

#[cfg(feature = "roym")]
pub(super) async fn init_roym(
    shared: &SharedNodeHandles,
    endpoint_registry: &EndpointRegistry,
    node_service_id: &str,
    config: &SubstrateConfig,
) -> anyhow::Result<Vec<Arc<syneroym_app_host_native::NativeHostFactory>>> {
    use std::{collections::BTreeSet, fs, sync::Weak, time::Duration};

    use syneroym_app_host_native::{
        ConversationSink, HttpSink, NativeHostFactory, NativeHttpAdapter, WebSocketSink,
    };
    use syneroym_app_orchestration::{
        AppInstanceId, LogicalServiceName, TopologyEntry, TopologyEpoch, TopologyKey, TopologyMode,
        models::ServiceId,
    };
    use syneroym_control_plane::assets;
    use syneroym_core::{asset_manifest::ServiceAssets, local_registry::SubstrateEndpoint};
    use syneroym_roym_core::services;
    use syneroym_rpc::{NativeHttpService, NativeService};

    let mut factories = Vec::new();

    // 1. One factory + one NativeService per service.
    let factory_web = NativeHostFactory::new(
        roym_dispatch_id(services::WEB.name),
        shared.key_store.clone(),
        shared.storage_provider.clone(),
        shared.blob_provider.clone(),
        shared.messaging_broker.clone(),
        endpoint_registry.clone(),
        shared.logical_resolver.clone(),
        shared.conversation.clone(),
        shared.websocket_senders.clone(),
    );
    let f_web = factory_web.clone();
    let f_web_http = factory_web.clone();
    let web = Arc::new(syneroym_roym_web::native::NativeWeb::new(
        roym_dispatch_id(services::WEB.name),
        move |caller| f_web.host_for(caller),
        // A guest HTTP / websocket request is router ingress, so the
        // native shim builds a wire-origin host for it, matching the WASM
        // engine's unconditional `from_wire` on every guest HTTP request.
        move |caller| f_web_http.host_for_wire(caller),
    ));
    shared
        .native_dispatch
        .insert(roym_dispatch_id(services::WEB.name), web.clone() as Arc<dyn NativeService>);
    endpoint_registry
        .register(
            roym_dispatch_id(services::WEB.name),
            services::WEB.interface.to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: roym_dispatch_id(services::WEB.name),
            },
        )
        .await?;
    factories.push(factory_web.clone());

    let factory_profile = NativeHostFactory::new(
        roym_dispatch_id(services::PROFILE.name),
        shared.key_store.clone(),
        shared.storage_provider.clone(),
        shared.blob_provider.clone(),
        shared.messaging_broker.clone(),
        endpoint_registry.clone(),
        shared.logical_resolver.clone(),
        shared.conversation.clone(),
        shared.websocket_senders.clone(),
    );
    let f_profile = factory_profile.clone();
    let profile = Arc::new(syneroym_roym_profile::native::NativeProfile::new(
        roym_dispatch_id(services::PROFILE.name),
        move |caller| f_profile.host_for(caller),
    ));
    shared.native_dispatch.insert(
        roym_dispatch_id(services::PROFILE.name),
        profile.clone() as Arc<dyn NativeService>,
    );
    endpoint_registry
        .register(
            roym_dispatch_id(services::PROFILE.name),
            services::PROFILE.interface.to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: roym_dispatch_id(services::PROFILE.name),
            },
        )
        .await?;
    factories.push(factory_profile);

    let factory_conv = NativeHostFactory::new(
        roym_dispatch_id(services::CONVERSATION.name),
        shared.key_store.clone(),
        shared.storage_provider.clone(),
        shared.blob_provider.clone(),
        shared.messaging_broker.clone(),
        endpoint_registry.clone(),
        shared.logical_resolver.clone(),
        shared.conversation.clone(),
        shared.websocket_senders.clone(),
    );
    let f_conv = factory_conv.clone();
    let conv = Arc::new(syneroym_roym_conversation::native::NativeConversation::new(
        roym_dispatch_id(services::CONVERSATION.name),
        move |caller| f_conv.host_for(caller),
    ));
    // The delivery worker wakes the natively linked inbox the same way
    // `AppSandboxEngine` wakes a wasm-hosted one -- without this an
    // inbound message reaches the host store and stops there.
    factory_conv.set_conversation_sink(Arc::downgrade(&conv) as Weak<dyn ConversationSink>);
    shared.native_dispatch.insert(
        roym_dispatch_id(services::CONVERSATION.name),
        conv.clone() as Arc<dyn NativeService>,
    );
    endpoint_registry
        .register(
            roym_dispatch_id(services::CONVERSATION.name),
            services::CONVERSATION.interface.to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: roym_dispatch_id(services::CONVERSATION.name),
            },
        )
        .await?;
    factories.push(factory_conv);

    let factory_cat = NativeHostFactory::new(
        roym_dispatch_id(services::CATALOG.name),
        shared.key_store.clone(),
        shared.storage_provider.clone(),
        shared.blob_provider.clone(),
        shared.messaging_broker.clone(),
        endpoint_registry.clone(),
        shared.logical_resolver.clone(),
        shared.conversation.clone(),
        shared.websocket_senders.clone(),
    );
    let f_cat = factory_cat.clone();
    let cat = Arc::new(syneroym_roym_catalog::native::NativeCatalog::new(
        roym_dispatch_id(services::CATALOG.name),
        move |caller| f_cat.host_for(caller),
    ));
    shared
        .native_dispatch
        .insert(roym_dispatch_id(services::CATALOG.name), cat.clone() as Arc<dyn NativeService>);
    endpoint_registry
        .register(
            roym_dispatch_id(services::CATALOG.name),
            services::CATALOG.interface.to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: roym_dispatch_id(services::CATALOG.name),
            },
        )
        .await?;
    factories.push(factory_cat);

    let factory_tx = NativeHostFactory::new(
        roym_dispatch_id(services::TRANSACTION.name),
        shared.key_store.clone(),
        shared.storage_provider.clone(),
        shared.blob_provider.clone(),
        shared.messaging_broker.clone(),
        endpoint_registry.clone(),
        shared.logical_resolver.clone(),
        shared.conversation.clone(),
        shared.websocket_senders.clone(),
    );
    let f_tx = factory_tx.clone();
    let tx = Arc::new(syneroym_roym_transaction::native::NativeTransaction::new(
        roym_dispatch_id(services::TRANSACTION.name),
        move |caller| f_tx.host_for(caller),
    ));
    shared
        .native_dispatch
        .insert(roym_dispatch_id(services::TRANSACTION.name), tx.clone() as Arc<dyn NativeService>);
    endpoint_registry
        .register(
            roym_dispatch_id(services::TRANSACTION.name),
            services::TRANSACTION.interface.to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: roym_dispatch_id(services::TRANSACTION.name),
            },
        )
        .await?;
    factories.push(factory_tx);

    let factory_dir = NativeHostFactory::new(
        roym_dispatch_id(services::DIRECTORY.name),
        shared.key_store.clone(),
        shared.storage_provider.clone(),
        shared.blob_provider.clone(),
        shared.messaging_broker.clone(),
        endpoint_registry.clone(),
        shared.logical_resolver.clone(),
        shared.conversation.clone(),
        shared.websocket_senders.clone(),
    );
    let f_dir = factory_dir.clone();
    let dir = Arc::new(syneroym_roym_directory::native::NativeDirectory::new(
        roym_dispatch_id(services::DIRECTORY.name),
        move |caller| f_dir.host_for(caller),
    ));
    shared
        .native_dispatch
        .insert(roym_dispatch_id(services::DIRECTORY.name), dir.clone() as Arc<dyn NativeService>);
    endpoint_registry
        .register(
            roym_dispatch_id(services::DIRECTORY.name),
            services::DIRECTORY.interface.to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: roym_dispatch_id(services::DIRECTORY.name),
            },
        )
        .await?;
    factories.push(factory_dir);

    // 2. `web` alone gets the HTTP surface.
    let web_id = roym_dispatch_id("web");
    factory_web.set_http_sink(Arc::downgrade(&web) as Weak<dyn HttpSink>);
    factory_web.set_websocket_sink(Arc::downgrade(&web) as Weak<dyn WebSocketSink>);
    let adapter = Arc::new(NativeHttpAdapter::new(
        factory_web.clone(),
        Arc::downgrade(&web) as Weak<dyn HttpSink>,
        Arc::downgrade(&web) as Weak<dyn WebSocketSink>,
    ));
    shared.native_http.insert(web_id.clone(), adapter.clone() as Arc<dyn NativeHttpService>);
    shared.native_http.insert(node_service_id.to_string(), adapter as Arc<dyn NativeHttpService>);
    shared.http_routes.insert(web_id.clone(), roym_http_routes());
    shared.http_routes.insert(node_service_id.to_string(), roym_http_routes());
    endpoint_registry
        .register(
            web_id.clone(),
            syneroym_core::local_registry::HTTP_NATIVE_INTERFACE.to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: web_id.clone() },
        )
        .await?;
    endpoint_registry
        .register(
            node_service_id.to_string(),
            syneroym_core::local_registry::HTTP_NATIVE_INTERFACE.to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: web_id.clone() },
        )
        .await?;

    // 3. App context and dependency bindings.
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
        shared.logical_resolver.register(
            TopologyKey::local(
                AppInstanceId::new(ROYM_APP_INSTANCE),
                LogicalServiceName::new(dep.name),
            ),
            entry.clone(),
        );
        endpoint_registry
            .save_binding(&web_id, ROYM_APP_INSTANCE, dep.name, &serde_json::to_string(&entry)?)
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

    // 4. The UI bundle.
    if let Some(path) = config.roles.roym.as_ref().and_then(|r| r.ui_bundle_path.as_ref()) {
        match fs::read(path) {
            Ok(archive) => {
                // A DEK load failure must not silently downgrade to
                // unpacking the bundle unencrypted -- skip bundle
                // registration and log a warning instead, the same way
                // an unpack or manifest-store failure below does.
                match shared.storage_provider.load_service_dek(&web_id, &shared.key_store).await {
                    Ok(dek) => {
                        let mut written = BTreeSet::new();
                        let manifest = assets::unpack_asset_bundle(
                            &web_id,
                            &archive,
                            None,
                            &roym_http_routes(),
                            &shared.blob_provider,
                            dek.clone(),
                            &mut written,
                        )
                        .await;
                        match manifest {
                            Ok(m) => {
                                match assets::store_manifest(
                                    &web_id,
                                    &m,
                                    &shared.blob_provider,
                                    dek,
                                )
                                .await
                                {
                                    Ok(manifest_hash) => {
                                        shared.assets.insert(
                                            web_id.clone(),
                                            ServiceAssets {
                                                manifest: Arc::new(m),
                                                public: true,
                                                manifest_hash,
                                            },
                                        );
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            ?path,
                                            %e,
                                            "Roym UI bundle manifest could not be stored; serving \
                                             API without Hub"
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    ?path,
                                    %e,
                                    "Roym UI bundle unpack failed; serving API without Hub"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            ?path,
                            %e,
                            "Roym UI bundle's service DEK could not be loaded; serving API \
                             without Hub rather than unpacking it unencrypted"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(?path, %e, "Roym UI bundle could not be read; serving API without Hub");
            }
        }
    } else {
        tracing::info!("no roym.ui_bundle_path configured; serving the API without the Hub");
    }

    for factory in &factories {
        factory.set_record_signer(shared.record_signer.clone());
    }

    if let Some(owner_did) = config.roles.roym.as_ref().and_then(|r| r.owner_did.as_ref()) {
        syneroym_identity::substrate::resolve_did_key(owner_did)
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
