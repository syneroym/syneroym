//! [`NativeHostFactory`]: everything the shim needs that outlives one call.

use std::{
    fmt,
    sync::{Arc, OnceLock, Weak},
};

use dashmap::DashMap;
use syneroym_app_host::{MessageSink, types::messaging::MessagingError};
use syneroym_core::local_registry::EndpointRegistry;
use syneroym_data_blob::traits::BlobProvider;
use syneroym_data_db::traits::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, SubscriptionHandle, namespace_topic};
use syneroym_rpc::CallerContext;
use syneroym_sandbox_wasm::{HostState, MessagingContext, StreamContext, empty_service_proxy};

use crate::host::{HostInner, NativeAppHost};

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
    logical_resolver: Arc<syneroym_app_orchestration::LogicalResolver>,
    /// Live broker subscriptions, keyed by *namespaced* topic -- the native
    /// analogue of `AppSandboxEngine.subscriptions`.
    subscriptions: DashMap<String, SubscriptionHandle>,
    /// The app's inbound message entry point. `Weak`, not `Arc`: the app
    /// holds this factory, so a strong reference back would be the same
    /// uncollectable cycle `HostState.service_proxy` already guards against.
    sink: OnceLock<Weak<dyn MessageSink>>,
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
    ) -> Arc<Self> {
        Arc::new(Self {
            service_id,
            key_store,
            storage_provider,
            blob_provider,
            broker,
            endpoint_registry,
            logical_resolver,
            subscriptions: DashMap::new(),
            sink: OnceLock::new(),
        })
    }

    /// Sets the app's inbound message entry point. Panics if called twice --
    /// the app and the factory are constructed in a fixed order, exactly as
    /// `ControlPlaneService.service_proxy` is.
    #[allow(clippy::expect_used)]
    pub fn set_sink(&self, sink: Weak<dyn MessageSink>) {
        self.sink.set(sink).expect("NativeHostFactory::set_sink called more than once");
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
        let state = HostState::new(
            self.service_id.clone(),
            None, // max_memory_bytes: no wasm memory to bound
            self.key_store.clone(),
            self.storage_provider.clone(),
            self.blob_provider.clone(),
            caller,
            0, // config_generation
            MessagingContext { broker: self.broker.clone(), engine: Weak::new() },
            StreamContext { registry: self.endpoint_registry.clone(), engine: Weak::new() },
            empty_service_proxy(),
            None, // fdae_policy: a linked app has no deploy record
            read_only,
            syneroym_rpc::empty_row_authorizer(),
            None, // app_instance_id
            self.logical_resolver.clone(),
        );
        NativeAppHost::new(Arc::new(HostInner {
            factory: self.clone(),
            state: tokio::sync::Mutex::new(state),
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

        let factory = NativeHostFactory::new(
            "read-only-unit-test".to_string(),
            key_store,
            storage_provider,
            blob_provider,
            broker,
            endpoint_registry,
            syneroym_app_orchestration::empty_resolver(),
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
