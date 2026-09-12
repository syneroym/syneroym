//! Test fixture for dual-build shim.

#[cfg(feature = "dual_build_fixture")]
use std::sync::Arc;

#[cfg(feature = "dual_build_fixture")]
use syneroym_core::local_registry::{EndpointRegistry, SubstrateEndpoint};
#[cfg(feature = "dual_build_fixture")]
use syneroym_rpc::NativeService;

#[cfg(feature = "dual_build_fixture")]
use super::router::SharedNodeHandles;

/// The reserved `native_dispatch` key the dual-build-shim fixture registers
/// under, independent of this node's own DID -- the same shape
/// `SUPERVISOR_DISPATCH_ID` uses.
#[cfg(feature = "dual_build_fixture")]
pub(super) const DUAL_BUILD_FIXTURE_DISPATCH_ID: &str = "dual-build-fixture";

/// Links the dual-build-shim fixture's native build in as a
/// `NativeService`, proving the shim works end to end: built both ways from
/// one source tree, linked into `syneroym-substrate` behind a Cargo
/// feature. Not a deployed service: there is no undeploy path, so
/// `NativeHostFactory::shutdown` is never called here -- dropping the
/// factory at process exit is the real teardown, and `SubscriptionHandle`'s
/// own `Drop` unsubscribes.
///
/// Registers under the node's own DID with no access control of its own --
/// see the `dual_build_fixture` feature's own comment in `Cargo.toml`. Test
/// scaffolding only; never enable this feature in a release or deploy
/// profile.
#[cfg(feature = "dual_build_fixture")]
pub(super) async fn init_dual_build_fixture(
    shared: &SharedNodeHandles,
    endpoint_registry: &EndpointRegistry,
    node_service_id: &str,
) -> anyhow::Result<Option<Arc<syneroym_app_host_native::NativeHostFactory>>> {
    use std::sync::Weak;

    use syneroym_app_host_native::{
        HttpSink, MessageSink, NativeHostFactory, NativeHttpAdapter, WebSocketSink,
    };
    use syneroym_core::http_routes::HttpRoute;
    use syneroym_rpc::NativeHttpService;
    use syneroym_test_dual_build_fixture::native::{FIXTURE_INTERFACE, NativeFixture};

    let service_id = DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string();
    let factory = NativeHostFactory::new(
        service_id.clone(),
        shared.key_store.clone(),
        shared.storage_provider.clone(),
        shared.blob_provider.clone(),
        shared.messaging_broker.clone(),
        endpoint_registry.clone(),
        shared.logical_resolver.clone(),
        shared.conversation.clone(),
        shared.websocket_senders.clone(),
    );
    factory.set_record_signer(shared.record_signer.clone());
    let f = factory.clone();
    let f_http = factory.clone();
    let fixture = Arc::new(NativeFixture::new(
        service_id.clone(),
        move |caller| f.host_for(caller),
        move |caller| f_http.host_for_wire(caller),
    ));
    factory.set_sink(Arc::downgrade(&fixture) as Weak<dyn MessageSink>);
    factory.set_conversation_sink(
        Arc::downgrade(&fixture) as Weak<dyn syneroym_app_host_native::ConversationSink>
    );
    factory.set_http_sink(Arc::downgrade(&fixture) as Weak<dyn HttpSink>);
    factory.set_websocket_sink(Arc::downgrade(&fixture) as Weak<dyn WebSocketSink>);

    shared.native_dispatch.insert(
        DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(),
        fixture.clone() as Arc<dyn NativeService>,
    );

    let adapter = Arc::new(NativeHttpAdapter::new(
        factory.clone(),
        Arc::downgrade(&fixture) as Weak<dyn HttpSink>,
        Arc::downgrade(&fixture) as Weak<dyn WebSocketSink>,
    ));
    shared.native_http.insert(
        DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(),
        adapter.clone() as Arc<dyn NativeHttpService>,
    );
    shared.native_http.insert(node_service_id.to_string(), adapter as Arc<dyn NativeHttpService>);
    let routes = vec![
        HttpRoute {
            method: "POST".into(),
            path: "/run".into(),
            target: "guest".into(),
            operation: "handle-request".into(),
            collection: None,
            topic: None,
            protocol: None,
            public: false,
        },
        HttpRoute {
            method: "POST".into(),
            path: "/store".into(),
            target: "guest".into(),
            operation: "handle-request".into(),
            collection: None,
            topic: None,
            protocol: None,
            public: false,
        },
        HttpRoute {
            method: "GET".into(),
            path: "/whoami".into(),
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
            public: false,
        },
        HttpRoute {
            method: "GET".into(),
            path: "/ws-public".into(),
            target: "websocket".into(),
            operation: "handle-upgrade".into(),
            collection: None,
            topic: None,
            protocol: None,
            public: true,
        },
    ];
    shared.http_routes.insert(DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(), routes.clone());
    shared.http_routes.insert(node_service_id.to_string(), routes);

    // Exactly one endpoint. Do not also register a `messaging` endpoint:
    // `EndpointRegistry::register` is a silent last-write-wins insert on
    // `(service_id, interface_name)`, `(node_did, "messaging")` already
    // belongs to the supervisor above, and CI builds `--all-features`, so
    // both would be live at once and one would quietly disappear. The
    // fixture needs nothing from that key: its `subscribe` is app-initiated
    // and its pump reads the broker directly, never through the router's
    // messaging path.
    endpoint_registry
        .register(
            node_service_id.to_string(),
            FIXTURE_INTERFACE.to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(),
            },
        )
        .await?;

    endpoint_registry
        .register(
            node_service_id.to_string(),
            "http".to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(),
            },
        )
        .await?;

    endpoint_registry
        .register(
            node_service_id.to_string(),
            "http-native".to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(),
            },
        )
        .await?;

    endpoint_registry
        .register(
            DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(),
            "http-native".to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(),
            },
        )
        .await?;

    Ok(Some(factory))
}
