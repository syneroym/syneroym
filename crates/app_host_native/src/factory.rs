//! [`NativeHostFactory`]: everything the shim needs that outlives one call.

use std::{
    fmt,
    sync::{Arc, OnceLock, Weak},
};

use dashmap::DashMap;
use syneroym_app_host::{ConversationSink, MessageSink, types::messaging::MessagingError};
use syneroym_app_orchestration::LogicalResolver;
use syneroym_conversation::ConversationService;
use syneroym_core::local_registry::EndpointRegistry;
use syneroym_data_blob::traits::BlobProvider;
use syneroym_data_db::traits::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_fdae::Policy;
use syneroym_mqtt_broker::{MqttBroker, SubscriptionHandle, namespace_topic};
use syneroym_rpc::{
    CallerContext, ConversationDeliveryState, ConversationHost, ConversationMessage,
    ConversationNotifier, ServiceProxy, WebSocketSenders,
};
use syneroym_sandbox_wasm::{HostState, MessagingContext, StreamContext, empty_service_proxy};
use tokio::sync::RwLock;

use crate::{
    convert,
    host::{HostInner, NativeAppHost},
    http::{HttpSink, WebSocketSink},
};

/// Everything the shim needs that outlives one call. One per native app
/// instance, held by whoever registered that app.
pub struct NativeHostFactory {
    /// Supplied by the embedder, never defaulted: it selects the service's
    /// SQLite store, its broker topic namespace, and the resource its
    /// `data-layer/admin` gate checks against.
    service_id: String,
    key_store: Arc<KeyStore>,
    storage_provider: Arc<dyn StorageProvider>,
    blob_provider: Arc<dyn BlobProvider>,
    broker: Arc<MqttBroker>,
    endpoint_registry: EndpointRegistry,
    logical_resolver: Arc<LogicalResolver>,
    /// Live broker subscriptions, keyed by *namespaced* topic -- the native
    /// analogue of `AppSandboxEngine.subscriptions`.
    subscriptions: DashMap<String, SubscriptionHandle>,
    /// The app's inbound message entry point. `Weak`, not `Arc`: the app
    /// holds this factory, so a strong reference back would be the same
    /// uncollectable cycle `HostState.service_proxy` already guards against.
    sink: OnceLock<Weak<dyn MessageSink>>,
    /// The Conversation service -- held strong: unlike
    /// `service_proxy`, `ConversationService` holds no reference back to
    /// this factory *directly* (it reaches it only through the `Weak`
    /// registered in `register_service_notifier` below), so there is no
    /// cycle to guard against.
    conversation: Arc<ConversationService>,
    /// The app's inbound conversation entry point, mirroring `sink` above.
    conversation_sink: OnceLock<Weak<dyn ConversationSink>>,
    service_proxy: OnceLock<Weak<dyn ServiceProxy>>,
    http_sink: OnceLock<Weak<dyn HttpSink>>,
    websocket_sink: OnceLock<Weak<dyn WebSocketSink>>,
    pub(crate) websocket_senders: Arc<WebSocketSenders>,
    fdae_policy: RwLock<Option<Option<Arc<Policy>>>>,
}

/// Hand-written, not derived: `StorageProvider` has no `Debug` supertrait,
/// which is the same reason `HostState` writes its own. Every other field
/// here does implement it, so `storage_provider` is the only one left out.
impl fmt::Debug for NativeHostFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeHostFactory")
            .field("service_id", &self.service_id)
            .field("subscriptions", &self.subscriptions.len())
            .finish_non_exhaustive()
    }
}

impl NativeHostFactory {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        service_id: String,
        key_store: Arc<KeyStore>,
        storage_provider: Arc<dyn StorageProvider>,
        blob_provider: Arc<dyn BlobProvider>,
        broker: Arc<MqttBroker>,
        endpoint_registry: EndpointRegistry,
        logical_resolver: Arc<syneroym_app_orchestration::LogicalResolver>,
        conversation: Arc<ConversationService>,
        websocket_senders: Arc<WebSocketSenders>,
    ) -> Arc<Self> {
        let factory = Arc::new(Self {
            service_id: service_id.clone(),
            key_store,
            storage_provider,
            blob_provider,
            broker,
            endpoint_registry,
            logical_resolver,
            subscriptions: DashMap::new(),
            sink: OnceLock::new(),
            conversation: conversation.clone(),
            conversation_sink: OnceLock::new(),
            service_proxy: OnceLock::new(),
            http_sink: OnceLock::new(),
            websocket_sink: OnceLock::new(),
            websocket_senders,
            fdae_policy: tokio::sync::RwLock::new(None),
        });
        // `§6.5`: registers itself as this service's conversation
        // notification target, so the delivery worker wakes a natively-
        // linked app the same way `AppSandboxEngine` wakes a wasm-hosted
        // one -- without this, the fixture's `on-message`/`on-delivery-
        // state` would fall through to `ConversationService`'s default
        // notifier (the wasm engine), which has no component for this
        // service id.
        conversation.register_service_notifier(
            service_id,
            Arc::downgrade(&factory) as Weak<dyn ConversationNotifier>,
        );
        factory
    }

    /// Sets the app's inbound message entry point. Panics if called twice --
    /// the app and the factory are constructed in a fixed order, exactly as
    /// `ControlPlaneService.service_proxy` is.
    #[allow(clippy::expect_used)]
    pub fn set_sink(&self, sink: Weak<dyn MessageSink>) {
        self.sink.set(sink).expect("NativeHostFactory::set_sink called more than once");
    }

    /// Sets the app's inbound conversation entry point, mirroring
    /// `set_sink`.
    #[allow(clippy::expect_used)]
    pub fn set_conversation_sink(&self, sink: Weak<dyn ConversationSink>) {
        self.conversation_sink
            .set(sink)
            .expect("NativeHostFactory::set_conversation_sink called more than once");
    }

    #[allow(clippy::expect_used)]
    pub fn set_service_proxy(&self, proxy: Weak<dyn ServiceProxy>) {
        self.service_proxy
            .set(proxy)
            .expect("NativeHostFactory::set_service_proxy called more than once");
    }

    #[allow(clippy::expect_used)]
    pub fn set_http_sink(&self, sink: Weak<dyn HttpSink>) {
        self.http_sink.set(sink).expect("NativeHostFactory::set_http_sink called more than once");
    }

    #[allow(clippy::expect_used)]
    pub fn set_websocket_sink(&self, sink: Weak<dyn WebSocketSink>) {
        self.websocket_sink
            .set(sink)
            .expect("NativeHostFactory::set_websocket_sink called more than once");
    }

    #[must_use]
    pub fn service_id(&self) -> &str {
        &self.service_id
    }

    /// One host handle for one invocation: a fresh `HostState`, exactly as
    /// the sandbox builds a fresh `Store` per guest call.
    #[must_use]
    pub fn host_for(self: &Arc<Self>, caller: CallerContext) -> NativeAppHost {
        self.host_with(caller, false)
    }

    pub(crate) async fn config_generation(&self) -> u64 {
        match self.storage_provider.get_latest_config_generation(&self.service_id).await {
            Ok(Some((g, _))) => g,
            Ok(None) => 0,
            Err(e) => {
                tracing::error!(
                    service_id = %self.service_id,
                    error = %e,
                    "failed to fetch config generation"
                );
                0
            }
        }
    }

    pub(crate) async fn fdae_policy(&self) -> Option<Arc<syneroym_fdae::Policy>> {
        {
            let guard = self.fdae_policy.read().await;
            if let Some(memoized) = &*guard {
                return memoized.clone();
            }
        }
        match self.storage_provider.load_fdae_policy(&self.service_id).await {
            Ok(Some(doc)) => match syneroym_fdae::parse_and_validate(&doc) {
                Ok(p) => {
                    let policy = Some(Arc::new(p));
                    let mut guard = self.fdae_policy.write().await;
                    *guard = Some(policy.clone());
                    policy
                }
                Err(e) => {
                    tracing::error!(
                        service_id = %self.service_id,
                        error = %e,
                        "FDAE policy failed to parse; treating as policy-absent"
                    );
                    let mut guard = self.fdae_policy.write().await;
                    *guard = Some(None);
                    None
                }
            },
            Ok(None) => {
                let mut guard = self.fdae_policy.write().await;
                *guard = Some(None);
                None
            }
            Err(e) => {
                tracing::error!(
                    service_id = %self.service_id,
                    error = %e,
                    "failed to load FDAE policy"
                );
                None
            }
        }
    }

    pub(crate) async fn build_host_state(
        &self,
        caller: CallerContext,
        read_only: bool,
    ) -> HostState {
        let service_proxy = self.service_proxy.get().cloned().unwrap_or_else(empty_service_proxy);
        let app_instance = self
            .endpoint_registry
            .app_context_of(&self.service_id)
            .map(|(instance, _name)| instance);
        let config_gen = self.config_generation().await;
        let fdae = self.fdae_policy().await;

        HostState::new(
            self.service_id.clone(),
            None, // max_memory_bytes: no wasm memory to bound
            self.key_store.clone(),
            self.storage_provider.clone(),
            self.blob_provider.clone(),
            caller,
            config_gen,
            MessagingContext { broker: self.broker.clone(), engine: Weak::new() },
            StreamContext { registry: self.endpoint_registry.clone(), engine: Weak::new() },
            service_proxy,
            fdae,
            read_only,
            syneroym_rpc::empty_row_authorizer(),
            app_instance,
            self.logical_resolver.clone(),
        )
        .with_conversation(Arc::downgrade(&self.conversation) as Weak<dyn ConversationHost>)
        .with_websocket_senders(self.websocket_senders.clone())
    }

    /// Private, and deliberately not `pub`: nothing an app or an embedder can
    /// reach may ask for a read-only host, because nothing native produces
    /// the stage-4 context that flag belongs to. It exists so this crate's
    /// own tests can prove the shim inherits the read-only denials rather
    /// than asserting it in prose.
    pub(crate) fn host_with(
        self: &Arc<Self>,
        caller: CallerContext,
        read_only: bool,
    ) -> NativeAppHost {
        NativeAppHost::new(Arc::new(HostInner {
            factory: self.clone(),
            caller,
            read_only,
            state: tokio::sync::OnceCell::new(),
        }))
    }

    /// Mirrors `host_api::Host::subscribe` + `register_internal_subscription`
    /// step for step, except that it does not persist the subscription: the
    /// substrate's boot-time replay (`replay_persisted_subscriptions`)
    /// hands every persisted row to the WASM engine regardless of which
    /// service id it names, so a native app's row would, after a restart,
    /// produce a live broker subscription pumping into a sandbox with no
    /// component to deliver to -- while the native subscription itself
    /// would not come back. A natively linked app's subscriptions therefore
    /// do not survive a restart (tracked in the deferred backlog).
    pub(crate) async fn subscribe(&self, topic: String) -> Result<(), MessagingError> {
        let namespaced = namespace_topic(&self.service_id, &topic);
        // Checked before the broker subscribe (rather than after) so a
        // teardown race never leaves a live broker registration with no
        // sink behind it -- same ordering `HostState::subscribe` uses for
        // its own engine-upgrade check.
        let weak_sink = self.sink.get().cloned().ok_or_else(|| {
            MessagingError::Internal("no message sink registered for this native app".to_string())
        })?;
        if self.subscriptions.contains_key(&namespaced) {
            return Ok(());
        }
        let (handle, mut rx) = self
            .broker
            .subscribe(namespaced.clone())
            .await
            .map_err(|e| MessagingError::Internal(e.to_string()))?;

        tokio::spawn(async move {
            while let Some((topic, payload)) = rx.recv().await {
                let Some(sink) = weak_sink.upgrade() else { break };
                // No component to instantiate natively, so nothing here
                // retries the way `deliver_message` retries wasm
                // instantiation: that retry exists to absorb wasmtime pool
                // pressure, not to add a delivery guarantee, so its absence
                // changes no observable contract.
                if let Err(e) = sink.handle_message(topic.clone(), payload).await {
                    tracing::warn!(%topic, error = %e, "native message delivery failed");
                }
            }
        });

        self.subscriptions.insert(namespaced, handle);
        Ok(())
    }

    pub(crate) fn unsubscribe(&self, topic: &str) -> Result<(), MessagingError> {
        let namespaced = namespace_topic(&self.service_id, topic);
        self.subscriptions.remove(&namespaced); // dropping `SubscriptionHandle` unsubscribes
        Ok(())
    }

    /// Drops every live subscription -- the explicit analogue of
    /// `AppSandboxEngine::unsubscribe_all`. A linked app has no undeploy
    /// path, so nothing in the substrate calls this; dropping the factory is
    /// the real teardown, and `SubscriptionHandle`'s own `Drop` unsubscribes.
    /// It exists for the parity suite, which tears one stack down while the
    /// process keeps running, and it is idempotent.
    pub fn shutdown(&self) {
        self.subscriptions.clear();
    }
}

/// The native build's wake mechanism: a `Weak<dyn ConversationSink>` with
/// no retry, unlike the WASM build's instantiate-and-call with a 4-attempt
/// retry (`AppSandboxEngine::notify_guest_message`) -- a stated, permitted
/// difference (B3's own precedent for `MessageSink`): what must match is
/// the store contents afterward, not the delivery mechanism or its timing.
#[async_trait::async_trait]
impl ConversationNotifier for NativeHostFactory {
    async fn notify_message(&self, service_id: &str, msg: ConversationMessage) {
        debug_assert_eq!(
            service_id, self.service_id,
            "a factory only ever hears about its own service"
        );
        let Some(sink) = self.conversation_sink.get().and_then(Weak::upgrade) else { return };
        if let Err(e) = sink.on_message(convert::rpc_message_to_guest(msg)).await {
            tracing::warn!(service_id, error = %e, "native conversation on-message delivery failed");
        }
    }

    async fn notify_delivery_state(
        &self,
        service_id: &str,
        message_id: String,
        state: ConversationDeliveryState,
    ) {
        debug_assert_eq!(
            service_id, self.service_id,
            "a factory only ever hears about its own service"
        );
        let Some(sink) = self.conversation_sink.get().and_then(Weak::upgrade) else { return };
        if let Err(e) =
            sink.on_delivery_state(message_id, convert::rpc_delivery_state_to_guest(state)).await
        {
            tracing::warn!(service_id, error = %e, "native conversation on-delivery-state delivery failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use syneroym_app_host::{
        AppBlobStore, AppDataLayer, AppMessaging,
        types::{
            blob_store::BlobError,
            data_layer::{DataLayerError, RecordWriteValue},
            messaging::MessagingError,
        },
    };
    use syneroym_conversation::ConversationService;
    use syneroym_core::{local_registry::EndpointRegistry, storage::MockStorage};
    use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
    use syneroym_data_db::{SqliteStorageProvider, StorageProvider};
    use syneroym_data_keystore::KeyStore;
    use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
    use syneroym_rpc::{AuthLevel, CallerContext, SessionContext};

    use super::NativeHostFactory;

    fn caller() -> CallerContext {
        CallerContext {
            caller_did: "did:key:zReadOnlyTestCaller".to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: "did:key:zReadOnlyTestCaller".to_string(),
                ..Default::default()
            },
            auth: AuthLevel::Ucan,
            proof: None,
        }
    }

    /// `host_with`'s only reason to exist: proving the shim's `read_only`
    /// flag reaches every mutating/egress call through `HostState`'s own
    /// gate, the same way the WASM build's stage-4 after-step instances are
    /// denied. See the deferred-backlog row this test closes.
    #[tokio::test]
    async fn read_only_host_denies_every_mutating_and_egress_call() {
        let dir = tempfile::tempdir().unwrap();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(dir.path().join("data"), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let endpoint_registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let conversation = ConversationService::new(
            storage_provider.clone(),
            key_store.clone(),
            endpoint_registry.clone(),
            syneroym_async_queue::QueueConfig {
                retry: syneroym_core::config::RetryPolicy::default(),
                visibility_timeout_ms: 120_000,
                dlq_max_rows: 100,
                max_pending_rows: syneroym_async_queue::DEFAULT_MAX_PENDING_ROWS,
            },
            syneroym_conversation::ConversationConfig::default(),
        )
        .unwrap();
        let factory = NativeHostFactory::new(
            "read-only-unit-test".to_string(),
            key_store,
            storage_provider,
            blob_provider,
            broker,
            endpoint_registry,
            syneroym_app_orchestration::empty_resolver(),
            conversation,
            syneroym_rpc::WebSocketSenders::new(),
        );
        let host = factory.host_with(caller(), true);

        assert!(matches!(
            host.put(
                "docs".to_string(),
                RecordWriteValue { id: "id".to_string(), payload: vec![] },
            )
            .await,
            Err(DataLayerError::PermissionDenied)
        ));
        assert!(matches!(
            host.delete("docs".to_string(), "id".to_string()).await,
            Err(DataLayerError::PermissionDenied)
        ));
        assert!(matches!(
            host.publish("topic".to_string(), vec![]).await,
            Err(MessagingError::PermissionDenied)
        ));
        assert!(matches!(
            host.subscribe("topic".to_string()).await,
            Err(MessagingError::PermissionDenied)
        ));
        assert!(matches!(
            host.unsubscribe("topic".to_string()).await,
            Err(MessagingError::PermissionDenied)
        ));
        assert!(matches!(
            host.put_blob(vec![1, 2, 3]).await,
            Err(BlobError::Internal(msg)) if msg.contains("read-only")
        ));
        assert!(matches!(
            host.signed_url("somehash".to_string(), 60).await,
            Err(BlobError::Internal(msg)) if msg.contains("read-only")
        ));
    }
}
