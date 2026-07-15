//! Per-request WASM host state and the data-layer/vault/app-config/
//! blob-store/messaging host-capability implementations exposed to guests.
//!
//! Distinct from `engine`: this module wraps host-provided capabilities
//! (storage, secrets, config, blobs, messaging) that a guest reaches through
//! the WIT-generated `Host` traits. `engine` owns the wasmtime
//! compile/instantiate/run lifecycle those capabilities are wired into via
//! `AppSandboxEngine::build_wasm_linker`.

use std::{
    fmt::{self, Debug, Formatter},
    sync::{Arc, Weak},
    time::Duration,
};

use serde_json::Value;
use syneroym_core::local_registry::SubstrateEndpoint;
use syneroym_data_blob::{
    BlobError as BlobStoreError, HostDownloadSession, HostUploadSession, traits::BlobProvider,
};
use syneroym_data_db::traits::{ServiceStore, StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{
    MessagingError as BrokerMessagingError, MqttBroker, namespace_topic,
    namespace_topic_for_publish,
};
use syneroym_rpc::{
    Ability, CallOrigin, CallerContext, ProxyError as RpcProxyError, ProxyProtocol, ProxyRequest,
    ResourceUri, ServiceProxy,
};
use syneroym_wit_interfaces::host::syneroym::{
    app_config::app_config::{self, ConfigError},
    blob_store::blob_store::{
        self, BlobError, BlobReader, BlobWriter, HostBlobReader, HostBlobWriter,
    },
    data_layer::store::{
        self, CollectionSchema, DataLayerError, Mutation, QueryOptions, QueryResult,
        RawQueryResult, RecordReadValue, RecordWriteValue, SqlValue,
    },
    host::context::Host,
    messaging::host_api::{self, MessagingError},
    proxy::proxy,
    vault::vault::{self, VaultError},
};
use tracing::error;
use wasmtime::{StoreLimits, StoreLimitsBuilder, component::Resource};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxView, WasiView};
use zeroize::Zeroizing;

use crate::{engine::AppSandboxEngine, stream::StreamContext};

/// Bundles the messaging-specific pieces of `HostState`: the broker every
/// service shares, and a weak handle back to the owning `AppSandboxEngine`
/// so a live `subscribe()` call can register a delivery task that outlives
/// the ephemeral `Store`/`HostState` it was made from (every WASM
/// invocation gets a fresh `Store` -- see `AppSandboxEngine::self_weak`).
#[derive(Debug, Clone)]
pub struct MessagingContext {
    pub broker: Arc<MqttBroker>,
    pub engine: Weak<AppSandboxEngine>,
}

fn map_broker_error(e: BrokerMessagingError) -> MessagingError {
    match e {
        BrokerMessagingError::Internal(msg) => MessagingError::Internal(msg),
    }
}

/// An always-empty `Weak<dyn ServiceProxy>` (`.upgrade()` always returns
/// `None`) -- used before `AppSandboxEngine::service_proxy` has been set
/// (coordinator mode, or a test that never configures a proxy). The
/// inherent `Weak::new()` only exists for `T: Sized`, so an unsized `Weak<dyn
/// ServiceProxy>` has to be produced via Rust's unsized coercion from a
/// concrete, never-instantiated marker type instead.
pub fn empty_service_proxy() -> Weak<dyn ServiceProxy> {
    #[derive(Debug)]
    struct NeverConstructed;
    #[async_trait::async_trait]
    impl ServiceProxy for NeverConstructed {
        async fn invoke(&self, _request: ProxyRequest) -> Result<Value, RpcProxyError> {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
    }
    Weak::<NeverConstructed>::new()
}

/// Host state instantiated per-request for WASM components
pub struct HostState {
    pub wasi: WasiCtx,
    pub table: ResourceTable,
    // Custom state
    pub component_id: String,
    pub request_ctx: Option<String>,
    pub memory_limits: StoreLimits,
    pub key_store: Arc<KeyStore>,
    pub storage_provider: Arc<dyn StorageProvider>,
    pub blob_provider: Arc<dyn BlobProvider>,
    pub caller: CallerContext,
    pub config_generation: u64,
    pub messaging: MessagingContext,
    pub streaming: StreamContext,
    /// Weak handle to the Universal Proxy (M04A Slice A1), letting a guest
    /// originate a cross-service call via `syneroym:proxy/proxy::call`.
    /// `Weak`, not `Arc`: `ProxyRouter` (the only implementation) itself
    /// holds a `Weak<AppSandboxEngine>` back for local WASM targets, so two
    /// strong refs would form the same class of uncollectable cycle that
    /// hung graceful shutdown in Slice 6B.
    pub service_proxy: Weak<dyn ServiceProxy>,
}

impl Debug for HostState {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostState")
            .field("component_id", &self.component_id)
            .field("request_ctx", &self.request_ctx)
            .finish_non_exhaustive()
    }
}

impl HostState {
    /// Creates a new HostState with standard WASI context and storage provider
    /// references.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        component_id: String,
        max_memory_bytes: Option<usize>,
        key_store: Arc<KeyStore>,
        storage_provider: Arc<dyn StorageProvider>,
        blob_provider: Arc<dyn BlobProvider>,
        caller: CallerContext,
        config_generation: u64,
        messaging: MessagingContext,
        streaming: StreamContext,
        service_proxy: Weak<dyn ServiceProxy>,
    ) -> Self {
        let wasi = WasiCtx::builder().build();
        let table = ResourceTable::new();
        let memory_limits = StoreLimitsBuilder::new()
            .memory_size(max_memory_bytes.unwrap_or(usize::MAX))
            .instances(1)
            .memories(1)
            .tables(1)
            .build();
        Self {
            wasi,
            table,
            component_id,
            request_ctx: None,
            memory_limits,
            key_store,
            storage_provider,
            blob_provider,
            caller,
            config_generation,
            messaging,
            streaming,
            service_proxy,
        }
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl Host for HostState {
    async fn get_test_context(&mut self, request_ctx: String) -> String {
        let component_ctx = format!("Component: {}", self.component_id);
        if let Some(existing) = &self.request_ctx {
            format!("{component_ctx} | {existing} | {request_ctx}")
        } else {
            format!("{component_ctx} | {request_ctx}")
        }
    }
}

impl vault::Host for HostState {
    async fn reveal(&mut self, key: String) -> Result<Vec<u8>, VaultError> {
        let provider = self.storage_provider.clone();
        let key_store = self.key_store.clone();
        let service_id = self.component_id.clone();

        let store = match provider.open_service_db(&service_id, &key_store).await {
            Ok(s) => s,
            Err(e) => {
                error!(
                    "Vault reveal failed to open service DB for service_id {}: {}",
                    service_id, e
                );
                return Err(VaultError::Internal(e.to_string()));
            }
        };

        match store.reveal_secret(&key).await {
            Ok(Some(bytes)) => Ok(bytes),
            Ok(None) => Err(VaultError::NotFound),
            Err(e) => {
                error!("Vault reveal failed to read secret for service_id {}: {}", service_id, e);
                Err(VaultError::Internal(e.to_string()))
            }
        }
    }
}

impl host_api::Host for HostState {
    async fn publish(&mut self, topic: String, payload: Vec<u8>) -> Result<(), MessagingError> {
        let namespaced = namespace_topic_for_publish(&self.component_id, &topic);
        let broker = self.messaging.broker.clone();
        broker.publish(namespaced, payload).await.map_err(map_broker_error)
    }

    async fn subscribe(&mut self, topic: String) -> Result<(), MessagingError> {
        let namespaced = namespace_topic(&self.component_id, &topic);
        let service_id = self.component_id.clone();
        let storage_provider = self.storage_provider.clone();
        let engine = self.messaging.engine.clone();

        // Checked before the DB write (rather than after) so a teardown
        // race never leaves a persisted subscription row with no live
        // broker registration behind it.
        let Some(engine) = engine.upgrade() else {
            return Err(MessagingError::Internal(
                "sandbox engine unavailable for subscription registration".to_string(),
            ));
        };

        storage_provider
            .save_messaging_subscription(&service_id, &namespaced)
            .await
            .map_err(|e| MessagingError::Internal(e.to_string()))?;

        engine
            .register_internal_subscription(&service_id, &namespaced)
            .await
            .map_err(|e| MessagingError::Internal(e.to_string()))
    }

    async fn unsubscribe(&mut self, topic: String) -> Result<(), MessagingError> {
        let namespaced = namespace_topic(&self.component_id, &topic);
        let service_id = self.component_id.clone();
        let storage_provider = self.storage_provider.clone();
        let engine = self.messaging.engine.clone();

        storage_provider
            .delete_messaging_subscription(&service_id, &namespaced)
            .await
            .map_err(|e| MessagingError::Internal(e.to_string()))?;

        // Surfaced as an error (rather than silently `Ok`) since the DB
        // row is already gone at this point: a caller told "success" here
        // while the live subscription stays active would have no way to
        // rediscover and clean it up later, via replay or otherwise.
        let Some(engine) = engine.upgrade() else {
            return Err(MessagingError::Internal(
                "sandbox engine unavailable for subscription deregistration".to_string(),
            ));
        };
        engine.subscriptions.remove(&(service_id, namespaced));
        Ok(())
    }

    async fn register_stream_protocol(&mut self, protocol: String) -> Result<(), MessagingError> {
        let service_id = self.component_id.clone();
        self.streaming
            .registry
            .register(service_id.clone(), protocol, SubstrateEndpoint::WasmChannel { service_id })
            .await
            .map_err(|e| MessagingError::Internal(e.to_string()))
    }
}

/// Opens the calling component's isolated `ServiceStore`, mapping any
/// storage-level failure into an `Internal` data-layer error.
///
/// Takes owned/cloned pieces rather than `&HostState`: `HostState` embeds a
/// `WasiCtx`, which is not `Sync`, so holding a `&HostState` across an
/// `.await` would make the enclosing future non-`Send` (required by the
/// generated `Host` trait). Callers must clone what they need out of `self`
/// before awaiting, exactly as the pre-existing `vault::reveal` impl below
/// already does.
async fn open_store(
    component_id: String,
    key_store: Arc<KeyStore>,
    storage_provider: Arc<dyn StorageProvider>,
) -> Result<Box<dyn ServiceStore>, DataLayerError> {
    storage_provider
        .open_service_db(&component_id, &key_store)
        .await
        .map_err(|e| DataLayerError::Internal(e.to_string()))
}

impl app_config::Host for HostState {
    async fn get(&mut self, key: String) -> Result<Option<String>, ConfigError> {
        if self.config_generation == 0 {
            return Ok(None);
        }

        let config_str = match self
            .storage_provider
            .get_config_generation(&self.component_id, self.config_generation)
            .await
        {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(None),
            Err(e) => {
                error!("Failed to read config for {}: {}", self.component_id, e);
                return Err(ConfigError::Internal(e.to_string()));
            }
        };

        let config_json: Value = match serde_json::from_str(&config_str) {
            Ok(j) => j,
            Err(e) => {
                error!("Invalid config JSON for {}: {}", self.component_id, e);
                return Err(ConfigError::Internal(e.to_string()));
            }
        };

        let val = config_json.get(&key).and_then(|v| v.as_str()).map(|s| s.to_string());
        Ok(val)
    }

    async fn get_section(&mut self, prefix: String) -> Result<Vec<(String, String)>, ConfigError> {
        if self.config_generation == 0 {
            return Ok(vec![]);
        }

        let config_str = match self
            .storage_provider
            .get_config_generation(&self.component_id, self.config_generation)
            .await
        {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(vec![]),
            Err(e) => {
                error!("Failed to read config for {}: {}", self.component_id, e);
                return Err(ConfigError::Internal(e.to_string()));
            }
        };

        let config_json: Value = match serde_json::from_str(&config_str) {
            Ok(j) => j,
            Err(e) => {
                error!("Invalid config JSON for {}: {}", self.component_id, e);
                return Err(ConfigError::Internal(e.to_string()));
            }
        };

        let mut results = vec![];
        if let Value::Object(map) = config_json {
            for (k, v) in map {
                #[allow(clippy::collapsible_if)]
                if k == prefix || k.starts_with(&format!("{prefix}.")) {
                    if let Some(s) = v.as_str() {
                        results.push((k, s.to_string()));
                    }
                }
            }
        }

        Ok(results)
    }
}

impl store::Host for HostState {
    async fn create_collection(&mut self, schema: CollectionSchema) -> Result<(), DataLayerError> {
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        store.create_collection(&schema).await
    }

    async fn drop_collection(&mut self, name: String) -> Result<(), DataLayerError> {
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        store.drop_collection(&name).await
    }

    async fn put(
        &mut self,
        collection: String,
        value: RecordWriteValue,
    ) -> Result<(), DataLayerError> {
        let creator_id = self.component_id.clone();
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        store.put(&collection, &value, &creator_id).await
    }

    async fn patch(
        &mut self,
        collection: String,
        id: String,
        patch_json: Vec<u8>,
    ) -> Result<(), DataLayerError> {
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        store.patch(&collection, &id, &patch_json).await
    }

    async fn get(
        &mut self,
        collection: String,
        id: String,
    ) -> Result<Option<RecordReadValue>, DataLayerError> {
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        store.get(&collection, &id).await
    }

    async fn query(
        &mut self,
        collection: String,
        opts: QueryOptions,
    ) -> Result<QueryResult, DataLayerError> {
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        store.query(&collection, &opts).await
    }

    async fn delete(&mut self, collection: String, id: String) -> Result<(), DataLayerError> {
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        store.delete(&collection, &id).await
    }

    async fn delete_many(
        &mut self,
        collection: String,
        filter: String,
    ) -> Result<u64, DataLayerError> {
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        store.delete_many(&collection, Some(filter.as_str())).await
    }

    async fn batch_mutate(
        &mut self,
        collection: String,
        mutations: Vec<Mutation>,
    ) -> Result<(), DataLayerError> {
        let creator_id = self.component_id.clone();
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        store.batch_mutate(&collection, &mutations, &creator_id).await
    }

    async fn execute_ddl(&mut self, sql: String) -> Result<(), DataLayerError> {
        // Admin-capability gate (ADR-0015/0016, replaces the former
        // `is_init_context` scaffold): only a caller holding
        // `data-layer/admin` on this component's own resource may run DDL.
        // Lifecycle init/migrate runs as `AuthLevel::LocalElevated`
        // (`CallerContext::local_elevated`), which carries it.
        let resource = ResourceUri::service(&self.component_id, &self.component_id);
        if !self.caller.has_capability(&resource, &Ability(Ability::DATA_LAYER_ADMIN.to_string())) {
            return Err(DataLayerError::PermissionDenied);
        }
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        store.execute_ddl(&sql).await
    }

    async fn query_raw(
        &mut self,
        sql: String,
        params: Vec<SqlValue>,
    ) -> Result<RawQueryResult, DataLayerError> {
        // Admin-capability gate (ADR-0015/0016), identical to execute_ddl: only
        // a caller holding `data-layer/admin` on this component's own resource
        // may run raw SQL. Lifecycle init/migrate runs as
        // `AuthLevel::LocalElevated`, which carries it.
        let resource = ResourceUri::service(&self.component_id, &self.component_id);
        if !self.caller.has_capability(&resource, &Ability(Ability::DATA_LAYER_ADMIN.to_string())) {
            return Err(DataLayerError::PermissionDenied);
        }
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        store.query_raw(&sql, &params).await
    }
}

/// Maps the proxy's transport-agnostic `syneroym_rpc::ProxyError` onto the
/// guest-facing `syneroym:proxy/proxy::proxy-error` WIT variant.
fn map_proxy_error(e: RpcProxyError) -> proxy::ProxyError {
    match e {
        RpcProxyError::ServiceNotFound(s) => proxy::ProxyError::ServiceNotFound(s),
        RpcProxyError::UnsupportedProtocol(s) => proxy::ProxyError::UnsupportedProtocol(s),
        RpcProxyError::UnsupportedTarget(s) => proxy::ProxyError::UnsupportedTarget(s),
        RpcProxyError::PermissionDenied(s) => proxy::ProxyError::PermissionDenied(s),
        RpcProxyError::Transport(s) => proxy::ProxyError::Transport(s),
        RpcProxyError::Timeout(_) => proxy::ProxyError::TimedOut,
        RpcProxyError::Callee { code, message, data } => {
            proxy::ProxyError::Callee(proxy::CalleeError {
                code,
                message,
                data: data.map(|v| v.to_string()),
            })
        }
        RpcProxyError::Internal(s) => proxy::ProxyError::Internal(s),
    }
}

impl proxy::Host for HostState {
    /// Originates a cross-service call through the Universal Proxy (M04A
    /// Slice A1). Always constructs `CallOrigin::Guest` -- this is the only
    /// construction site a component can reach, so the proxy's guest
    /// native-capability gate (`ProxyRouter::check_native_capability_gate`)
    /// cannot be bypassed from guest code.
    async fn call(
        &mut self,
        service: String,
        interface: String,
        method: String,
        params: String,
        options: Option<proxy::CallOptions>,
    ) -> Result<String, proxy::ProxyError> {
        let service_proxy = self
            .service_proxy
            .upgrade()
            .ok_or_else(|| proxy::ProxyError::Internal("proxy unavailable".to_string()))?;

        let params: Value = if params.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&params)
                .map_err(|e| proxy::ProxyError::Internal(format!("params must be JSON: {e}")))?
        };

        let (protocol_tag, idempotent, timeout_ms) = match &options {
            Some(o) => (o.protocol.as_deref(), o.idempotent, o.timeout_ms),
            None => (None, false, None),
        };
        let protocol =
            ProxyProtocol::parse(protocol_tag).map_err(proxy::ProxyError::UnsupportedProtocol)?;

        let req = ProxyRequest {
            target_service: service,
            interface,
            method,
            params,
            // The component acts as itself. It does NOT inherit the
            // identity of whoever invoked it (no U->X delegation exists in
            // B0's model), so a proxied call cannot be used to escalate to
            // the original caller's rights. Real caller-delegation is
            // B1/UCAN.
            caller: CallerContext::service_system(&self.component_id),
            origin: CallOrigin::Guest { service_id: self.component_id.clone() },
            protocol,
            idempotent,
            timeout: timeout_ms.map(|ms| Duration::from_millis(ms.into())),
        };

        let value = service_proxy.invoke(req).await.map_err(map_proxy_error)?;
        // Mirrors A0's boundary convention (a string result comes back raw,
        // not JSON-quoted) so guest code doesn't have to strip quotes.
        Ok(match value {
            Value::String(s) => s,
            other => other.to_string(),
        })
    }
}

fn map_blob_error(e: BlobStoreError) -> BlobError {
    match e {
        BlobStoreError::NotFound => BlobError::NotFound,
        BlobStoreError::QuotaExceeded => BlobError::QuotaExceeded,
        BlobStoreError::Internal(msg) => BlobError::Internal(msg),
    }
}

/// Resolves the calling component's DEK for blob encryption. `Ok(None)`
/// means `storage.encryption = false`; blobs are then stored in plaintext.
async fn resolve_blob_dek(
    component_id: &str,
    key_store: &Arc<KeyStore>,
    storage_provider: &Arc<dyn StorageProvider>,
) -> Result<Option<Zeroizing<[u8; 32]>>, BlobError> {
    storage_provider
        .load_service_dek(component_id, key_store)
        .await
        .map_err(|e| BlobError::Internal(e.to_string()))
}

impl blob_store::Host for HostState {
    async fn put_blob(&mut self, data: Vec<u8>) -> Result<String, BlobError> {
        let dek =
            resolve_blob_dek(&self.component_id, &self.key_store, &self.storage_provider).await?;
        self.blob_provider.put_blob(&self.component_id, data, dek).await.map_err(map_blob_error)
    }

    async fn get_blob(&mut self, hash: String) -> Result<Vec<u8>, BlobError> {
        let dek =
            resolve_blob_dek(&self.component_id, &self.key_store, &self.storage_provider).await?;
        self.blob_provider.get_blob(&self.component_id, &hash, dek).await.map_err(map_blob_error)
    }

    async fn open_upload(&mut self) -> Result<Resource<BlobWriter>, BlobError> {
        let dek =
            resolve_blob_dek(&self.component_id, &self.key_store, &self.storage_provider).await?;
        let session = self
            .blob_provider
            .open_upload(&self.component_id, dek)
            .await
            .map_err(map_blob_error)?;
        self.table.push(HostUploadSession(session)).map_err(|e| BlobError::Internal(e.to_string()))
    }

    async fn open_download(
        &mut self,
        hash: String,
        offset: u64,
    ) -> Result<Resource<BlobReader>, BlobError> {
        let dek =
            resolve_blob_dek(&self.component_id, &self.key_store, &self.storage_provider).await?;
        let session = self
            .blob_provider
            .open_download(&self.component_id, &hash, offset, dek)
            .await
            .map_err(map_blob_error)?;
        self.table
            .push(HostDownloadSession(session))
            .map_err(|e| BlobError::Internal(e.to_string()))
    }

    async fn delete_blob(&mut self, hash: String) -> Result<(), BlobError> {
        self.blob_provider.delete_blob(&self.component_id, &hash).await.map_err(map_blob_error)
    }

    async fn signed_url(&mut self, hash: String, ttl_secs: u32) -> Result<String, BlobError> {
        let dek =
            resolve_blob_dek(&self.component_id, &self.key_store, &self.storage_provider).await?;
        self.blob_provider
            .signed_url(&self.component_id, &hash, ttl_secs, dek)
            .await
            .map_err(map_blob_error)
    }
}

impl HostBlobWriter for HostState {
    async fn write(
        &mut self,
        self_: Resource<BlobWriter>,
        chunk: Vec<u8>,
    ) -> Result<(), BlobError> {
        let session = self.table.get_mut(&self_).map_err(|e| BlobError::Internal(e.to_string()))?;
        session.0.write(chunk).await.map_err(map_blob_error)
    }

    async fn finish(&mut self, self_: Resource<BlobWriter>) -> Result<String, BlobError> {
        let session = self.table.delete(self_).map_err(|e| BlobError::Internal(e.to_string()))?;
        session.0.finish().await.map_err(map_blob_error)
    }

    async fn abort(&mut self, self_: Resource<BlobWriter>) {
        if let Ok(session) = self.table.delete(self_) {
            session.0.abort().await;
        }
    }

    async fn drop(&mut self, rep: Resource<BlobWriter>) -> wasmtime::Result<()> {
        // If the guest dropped the resource without calling finish/abort,
        // discard whatever partial session state remains (implicit abort,
        // alongside the explicit `abort` method above).
        if let Ok(session) = self.table.delete(rep) {
            session.0.abort().await;
        }
        Ok(())
    }
}

impl HostBlobReader for HostState {
    async fn read(
        &mut self,
        self_: Resource<BlobReader>,
        max_bytes: u32,
    ) -> Result<Vec<u8>, BlobError> {
        let session = self.table.get_mut(&self_).map_err(|e| BlobError::Internal(e.to_string()))?;
        session.0.read(max_bytes).await.map_err(map_blob_error)
    }

    async fn drop(&mut self, rep: Resource<BlobReader>) -> wasmtime::Result<()> {
        let _ = self.table.delete(rep);
        Ok(())
    }
}

impl wasmtime::ResourceLimiter for HostState {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        match self.memory_limits.memory_growing(current, desired, maximum) {
            Ok(true) => Ok(true),
            _ => Err(wasmtime::Error::msg("MemoryFault: Wasm execution exceeded memory limit")),
        }
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        self.memory_limits.table_growing(current, desired, maximum)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use syneroym_core::{local_registry::EndpointRegistry, storage::MockStorage};
    use syneroym_data_blob::ObjectStoreBlobProvider;
    use syneroym_data_db::SqliteStorageProvider;
    use syneroym_mqtt_broker::MqttBrokerConfig;

    use super::*;

    /// Test-only blob provider: in-memory backend, effectively unlimited
    /// quota.
    pub(crate) fn test_blob_provider() -> Arc<dyn BlobProvider> {
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None))
    }

    /// Test-only messaging context: a real (but throwaway, no network
    /// listener) broker with no engine backreference -- sufficient for
    /// tests that don't exercise guest-delivery messaging.
    pub(crate) fn test_messaging_context() -> MessagingContext {
        MessagingContext {
            broker: Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
            engine: Weak::new(),
        }
    }

    /// Test-only streaming context: a mock in-memory `EndpointRegistry` with
    /// no engine backreference -- sufficient for tests that don't exercise
    /// stream-protocol registration/routing.
    pub(crate) fn test_streaming_context() -> StreamContext {
        StreamContext {
            registry: EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            engine: Weak::new(),
        }
    }

    /// Test-only proxy handle: always-unavailable -- sufficient for tests
    /// that don't exercise `syneroym:proxy/proxy::call`.
    pub(crate) fn test_service_proxy() -> Weak<dyn ServiceProxy> {
        super::empty_service_proxy()
    }

    #[tokio::test]
    async fn test_config_get_and_get_section() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());

        let config_json =
            r#"{"db_host": "localhost", "db_port": "5432", "db.password": "secret", "db": "mydb"}"#;
        let generation = storage.save_config_generation("test_svc", config_json).await.unwrap();

        let mut host = HostState::new(
            "test_svc".to_string(),
            None,
            Arc::new(KeyStore::new()),
            storage,
            test_blob_provider(),
            CallerContext::service_system("test-caller"),
            generation,
            test_messaging_context(),
            test_streaming_context(),
            test_service_proxy(),
        );

        use app_config::Host as ConfigHost;

        // 1. Existing key returns Ok(Some(value))
        let val = ConfigHost::get(&mut host, "db_host".to_string()).await.unwrap().unwrap();
        assert_eq!(val, "localhost");

        // 2. Missing key returns Ok(None)
        let missing = ConfigHost::get(&mut host, "db_user".to_string()).await.unwrap();
        assert!(missing.is_none());

        // get_section returns prefixed values with exact matching boundaries
        let section = ConfigHost::get_section(&mut host, "db".to_string()).await.unwrap();
        let mut section_keys: Vec<String> = section.into_iter().map(|(k, _)| k).collect();
        section_keys.sort();
        // "db" and "db.password" match. "db_host" and "db_port" DO NOT.
        assert_eq!(section_keys, vec!["db", "db.password"]);
    }

    #[tokio::test]
    async fn test_config_isolation_and_generation_pinning() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());

        // Service A Gen 1
        let gen1_a = storage.save_config_generation("svc_a", r#"{"mode": "v1"}"#).await.unwrap();
        // Service A Gen 2
        let gen2_a = storage.save_config_generation("svc_a", r#"{"mode": "v2"}"#).await.unwrap();

        // Service B Gen 1
        let gen1_b =
            storage.save_config_generation("svc_b", r#"{"mode": "b_mode"}"#).await.unwrap();

        use app_config::Host as ConfigHost;

        // Two WASM components with different configs get isolated values
        let mut host_a_gen2 = HostState::new(
            "svc_a".to_string(),
            None,
            Arc::new(KeyStore::new()),
            storage.clone(),
            test_blob_provider(),
            CallerContext::service_system("test-caller"),
            gen2_a,
            test_messaging_context(),
            test_streaming_context(),
            test_service_proxy(),
        );
        let mut host_b = HostState::new(
            "svc_b".to_string(),
            None,
            Arc::new(KeyStore::new()),
            storage.clone(),
            test_blob_provider(),
            CallerContext::service_system("test-caller"),
            gen1_b,
            test_messaging_context(),
            test_streaming_context(),
            test_service_proxy(),
        );

        let val_a = ConfigHost::get(&mut host_a_gen2, "mode".to_string()).await.unwrap().unwrap();
        let val_b = ConfigHost::get(&mut host_b, "mode".to_string()).await.unwrap().unwrap();
        assert_eq!(val_a, "v2");
        assert_eq!(val_b, "b_mode");

        // Re-deploy bumps generation; in-flight invocations retain prior generation
        let mut host_a_gen1 = HostState::new(
            "svc_a".to_string(),
            None,
            Arc::new(KeyStore::new()),
            storage.clone(),
            test_blob_provider(),
            CallerContext::service_system("test-caller"),
            gen1_a,
            test_messaging_context(),
            test_streaming_context(),
            test_service_proxy(),
        );
        let val_a_old =
            ConfigHost::get(&mut host_a_gen1, "mode".to_string()).await.unwrap().unwrap();
        assert_eq!(val_a_old, "v1");
    }

    /// M3A failure/security test: `vault/reveal` on a non-existent key
    /// returns `vault-error::not-found` at the WIT host-function boundary
    /// (not just `Ok(None)` one layer down at `ServiceStore::reveal_secret`,
    /// which `syneroym-data-db`'s own tests already cover).
    #[tokio::test]
    async fn test_vault_reveal_not_found_at_host_boundary() {
        let key_store = Arc::new(KeyStore::new());
        key_store.inject_kek([3u8; 32], None).unwrap();
        let temp_dir = tempfile::tempdir().unwrap();
        let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), true).unwrap());
        let mut host_state = HostState::new(
            "vault-not-found-svc".to_string(),
            None,
            key_store,
            storage_provider,
            test_blob_provider(),
            CallerContext::service_system("test-caller"),
            0,
            test_messaging_context(),
            test_streaming_context(),
            test_service_proxy(),
        );

        let result = vault::Host::reveal(&mut host_state, "does-not-exist".to_string()).await;
        assert!(matches!(result, Err(VaultError::NotFound)));
    }
}
