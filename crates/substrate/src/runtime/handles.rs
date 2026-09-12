//! Shared node handles threaded through per-role init functions.
//!
//! Lives in its own module so the dependency between role files and the
//! router is one-directional: role files → handles ← router, rather than a
//! cycle where every role file imported `super::router::SharedNodeHandles`
//! and `router.rs` called into those same files.

use std::sync::Arc;

use syneroym_app_orchestration::LogicalResolver;
use syneroym_conversation::ConversationService;
use syneroym_core::{
    asset_manifest::AssetRegistry, http_routes::HttpRouteRegistry, record_signer::NodeRecordSigner,
};
use syneroym_data_blob::BlobProvider;
use syneroym_data_db::traits::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_identity::Identity;
use syneroym_mqtt_broker::MqttBroker;
use syneroym_rpc::NativeDispatchRegistry;

/// Handles the supervisor role (and any future post-router role that must
/// act as a first-class dispatch target on this same node) needs, but
/// which are otherwise fully consumed by `build_route_handler_deps`'s
/// return value before this function's caller gets to see them again.
/// Built once, in `build_route_handler_deps`, since only it has all of
/// these in scope before they move into `RouteHandlerDeps`/
/// `ControlPlaneService`.
pub(super) struct SharedNodeHandles {
    key_store: Arc<KeyStore>,
    storage_provider: Arc<dyn StorageProvider>,
    native_dispatch: NativeDispatchRegistry,
    /// The identity a post-router role presents when it connects, as a
    /// client, to other substrates (ADR-0021 §8) — a second handle to the
    /// node's own key material, not a distinct identity.
    client_identity: Arc<Identity>,
    /// The same broker `AppSandboxEngine` and `ControlPlaneService`
    /// publish/subscribe through, so the supervisor's alert publication
    /// shares one broker with the rest of the node instead of standing up a
    /// second one.
    messaging_broker: Arc<MqttBroker>,
    /// The `dual_build_fixture` and `roym` roles' `NativeHostFactory` need
    /// the same blob backend and logical resolver `build_route_handler_deps`
    /// already built, rather than standing up their own.
    #[cfg_attr(all(not(feature = "dual_build_fixture"), not(feature = "roym")), allow(dead_code))]
    blob_provider: Arc<dyn BlobProvider>,
    #[cfg_attr(all(not(feature = "dual_build_fixture"), not(feature = "roym")), allow(dead_code))]
    logical_resolver: Arc<LogicalResolver>,
    /// Needed by `setup_router` to wire the real `ServiceProxy` in once
    /// `ConnectionRouter::init` has built it, and by the
    /// `dual_build_fixture`/`roym` roles' `NativeHostFactory`.
    conversation: Arc<ConversationService>,
    /// The per-service HTTP route table. A linked native app has no deploy
    /// record, so nothing else would ever put its routes here.
    #[cfg_attr(all(not(feature = "dual_build_fixture"), not(feature = "roym")), allow(dead_code))]
    http_routes: HttpRouteRegistry,
    /// The `guest`/`websocket` route targets' native registry.
    #[cfg_attr(all(not(feature = "dual_build_fixture"), not(feature = "roym")), allow(dead_code))]
    native_http: syneroym_rpc::NativeHttpRegistry,
    /// The shared live-WebSocket table (`AppSandboxEngine` holds the same
    /// `Arc`).
    #[cfg_attr(all(not(feature = "dual_build_fixture"), not(feature = "roym")), allow(dead_code))]
    websocket_senders: Arc<syneroym_rpc::WebSocketSenders>,
    /// The static asset table. A linked native app has no deploy record,
    /// so nothing else would put its UI bundle here.
    #[cfg_attr(not(feature = "roym"), allow(dead_code))]
    assets: AssetRegistry,
    #[cfg_attr(all(not(feature = "dual_build_fixture"), not(feature = "roym")), allow(dead_code))]
    record_signer: Arc<NodeRecordSigner>,
}

impl SharedNodeHandles {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        key_store: Arc<KeyStore>,
        storage_provider: Arc<dyn StorageProvider>,
        native_dispatch: NativeDispatchRegistry,
        client_identity: Arc<Identity>,
        messaging_broker: Arc<MqttBroker>,
        blob_provider: Arc<dyn BlobProvider>,
        logical_resolver: Arc<LogicalResolver>,
        conversation: Arc<ConversationService>,
        http_routes: HttpRouteRegistry,
        native_http: syneroym_rpc::NativeHttpRegistry,
        websocket_senders: Arc<syneroym_rpc::WebSocketSenders>,
        assets: AssetRegistry,
        record_signer: Arc<NodeRecordSigner>,
    ) -> Self {
        Self {
            key_store,
            storage_provider,
            native_dispatch,
            client_identity,
            messaging_broker,
            blob_provider,
            logical_resolver,
            conversation,
            http_routes,
            native_http,
            websocket_senders,
            assets,
            record_signer,
        }
    }

    pub(super) fn key_store(&self) -> &Arc<KeyStore> {
        &self.key_store
    }

    pub(super) fn storage_provider(&self) -> &Arc<dyn StorageProvider> {
        &self.storage_provider
    }

    pub(super) fn native_dispatch(&self) -> &NativeDispatchRegistry {
        &self.native_dispatch
    }

    pub(super) fn client_identity(&self) -> &Arc<Identity> {
        &self.client_identity
    }

    pub(super) fn messaging_broker(&self) -> &Arc<MqttBroker> {
        &self.messaging_broker
    }

    #[cfg_attr(all(not(feature = "dual_build_fixture"), not(feature = "roym")), allow(dead_code))]
    pub(super) fn blob_provider(&self) -> &Arc<dyn BlobProvider> {
        &self.blob_provider
    }

    #[cfg_attr(all(not(feature = "dual_build_fixture"), not(feature = "roym")), allow(dead_code))]
    pub(super) fn logical_resolver(&self) -> &Arc<LogicalResolver> {
        &self.logical_resolver
    }

    pub(super) fn conversation(&self) -> &Arc<ConversationService> {
        &self.conversation
    }

    #[cfg_attr(
        all(not(feature = "auth"), not(feature = "dual_build_fixture"), not(feature = "roym")),
        allow(dead_code)
    )]
    pub(super) fn http_routes(&self) -> &HttpRouteRegistry {
        &self.http_routes
    }

    #[cfg_attr(
        all(not(feature = "auth"), not(feature = "dual_build_fixture"), not(feature = "roym")),
        allow(dead_code)
    )]
    pub(super) fn native_http(&self) -> &syneroym_rpc::NativeHttpRegistry {
        &self.native_http
    }

    #[cfg_attr(all(not(feature = "dual_build_fixture"), not(feature = "roym")), allow(dead_code))]
    pub(super) fn websocket_senders(&self) -> &Arc<syneroym_rpc::WebSocketSenders> {
        &self.websocket_senders
    }

    #[cfg_attr(not(feature = "roym"), allow(dead_code))]
    pub(super) fn assets(&self) -> &AssetRegistry {
        &self.assets
    }

    #[cfg_attr(all(not(feature = "dual_build_fixture"), not(feature = "roym")), allow(dead_code))]
    pub(super) fn record_signer(&self) -> &Arc<NodeRecordSigner> {
        &self.record_signer
    }
}
