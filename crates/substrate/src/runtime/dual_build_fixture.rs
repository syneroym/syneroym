//! Test fixture for dual-build shim.

#[cfg(feature = "dual_build_fixture")]
use std::sync::Arc;

#[cfg(feature = "dual_build_fixture")]
use syneroym_core::{
    http_routes::HttpRoute,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
};
#[cfg(feature = "dual_build_fixture")]
use syneroym_rpc::NativeService;

#[cfg(feature = "dual_build_fixture")]
use super::handles::SharedNodeHandles;

/// The reserved `native_dispatch` key the dual-build-shim fixture registers
/// under, independent of this node's own DID — the same shape
/// `SUPERVISOR_DISPATCH_ID` uses.
///
/// Private (not `pub(super)`) because no other module references this
/// constant: unlike `SUPERVISOR_DISPATCH_ID`, which `router.rs` uses when
/// registering the supervisor and messaging endpoints before calling
/// `init_supervisor`, all uses of this constant are inside
/// `init_dual_build_fixture` itself.
#[cfg(feature = "dual_build_fixture")]
const DUAL_BUILD_FIXTURE_DISPATCH_ID: &str = "dual-build-fixture";

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
    use syneroym_rpc::NativeHttpService;
    use syneroym_test_dual_build_fixture::native::{FIXTURE_INTERFACE, NativeFixture};

    let service_id = DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string();
    let factory = NativeHostFactory::new(
        service_id.clone(),
        shared.key_store().clone(),
        shared.storage_provider().clone(),
        shared.blob_provider().clone(),
        shared.messaging_broker().clone(),
        endpoint_registry.clone(),
        shared.logical_resolver().clone(),
        shared.conversation().clone(),
        shared.websocket_senders().clone(),
    );
    factory.set_record_signer(shared.record_signer().clone());
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

    shared.native_dispatch().insert(
        DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(),
        fixture.clone() as Arc<dyn NativeService>,
    );

    let adapter = Arc::new(NativeHttpAdapter::new(
        factory.clone(),
        Arc::downgrade(&fixture) as Weak<dyn HttpSink>,
        Arc::downgrade(&fixture) as Weak<dyn WebSocketSink>,
    ));
    shared.native_http().insert(
        DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(),
        adapter.clone() as Arc<dyn NativeHttpService>,
    );
    shared.native_http().insert(node_service_id.to_string(), adapter as Arc<dyn NativeHttpService>);
    let routes = dual_build_fixture_routes();
    shared.http_routes().insert(DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(), routes.clone());
    shared.http_routes().insert(node_service_id.to_string(), routes);

    // Exactly one endpoint. Do not also register a `messaging` endpoint:
    // `EndpointRegistry::register` is a silent last-write-wins insert on
    // `(service_id, interface_name)`, `(node_did, "messaging")` already
    // belongs to the supervisor above, and CI builds `--all-features`, so
    // both would be live at once and one would quietly disappear. The
    // fixture needs nothing from that key: its `subscribe` is app-initiated
    // and its pump reads the broker directly, never through the router's
    // messaging path.
    register_fixture_endpoint(endpoint_registry, node_service_id, FIXTURE_INTERFACE).await?;
    register_fixture_endpoint(endpoint_registry, node_service_id, "http").await?;
    register_fixture_endpoint(endpoint_registry, node_service_id, "http-native").await?;
    register_fixture_endpoint(endpoint_registry, DUAL_BUILD_FIXTURE_DISPATCH_ID, "http-native")
        .await?;

    Ok(Some(factory))
}

/// The fixture's five HTTP routes. `/run`/`/store` are private (only a
/// caller with a verified identity may reach them), `/whoami` and both
/// WebSocket upgrades are public — see `fixture_route`'s own doc for why
/// `collection`/`topic`/`protocol` never vary here.
#[cfg(feature = "dual_build_fixture")]
fn dual_build_fixture_routes() -> Vec<HttpRoute> {
    vec![
        fixture_route("POST", "/run", "guest", "handle-request", false),
        fixture_route("POST", "/store", "guest", "handle-request", false),
        fixture_route("GET", "/whoami", "guest", "handle-request", true),
        fixture_route("GET", "/ws", "websocket", "handle-upgrade", false),
        fixture_route("GET", "/ws-public", "websocket", "handle-upgrade", true),
    ]
}

/// Builds one fixture route entry. None of the five routes use
/// `collection`/`topic`/`protocol` (those only apply to the `data-layer`/
/// `messaging`/`stream` targets, and every fixture route targets `guest` or
/// `websocket`), so only `method`, `path`, `target`, `operation`, and
/// `public` need to vary per call.
#[cfg(feature = "dual_build_fixture")]
fn fixture_route(
    method: &str,
    path: &str,
    target: &str,
    operation: &str,
    public: bool,
) -> HttpRoute {
    HttpRoute {
        method: method.into(),
        path: path.into(),
        target: target.into(),
        operation: operation.into(),
        collection: None,
        topic: None,
        protocol: None,
        public,
    }
}

/// Registers one of the fixture's endpoint-registry entries. Every entry
/// resolves to the same `NativeHostChannel` (the fixture's own dispatch id)
/// — only the registering service id and interface name change per call, so
/// this is a thin wrapper around `EndpointRegistry::register` rather than a
/// copy of the same three-argument call four times.
#[cfg(feature = "dual_build_fixture")]
async fn register_fixture_endpoint(
    endpoint_registry: &EndpointRegistry,
    registrant: &str,
    interface: &str,
) -> anyhow::Result<()> {
    endpoint_registry
        .register(
            registrant.to_string(),
            interface.to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(),
            },
        )
        .await
}
