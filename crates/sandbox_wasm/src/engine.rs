//! WASM execution engine based on Wasmtime
//!
//! Sets up the sandboxed environment with strict CPU/memory quotas,
//! registers host capabilities, and runs WASM component binaries.

use std::{
    fmt::{self, Debug, Formatter},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, Weak},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_core::{config::SubstrateConfig, local_registry::SubstrateEndpoint};
use syneroym_data_blob::{
    BlobError as BlobStoreError, HostDownloadSession, HostUploadSession, traits::BlobProvider,
};
use syneroym_data_db::traits::{ServiceStore, StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{
    MessagingError as BrokerMessagingError, MqttBroker, SubscriptionHandle, namespace_topic,
};
use syneroym_rpc::JsonRpcRequest;
use syneroym_wit_interfaces::{
    control_plane::exports::syneroym::control_plane::orchestrator::{
        ArtifactSource, DeployManifest, ServiceType,
    },
    host::syneroym::{
        app_config::app_config::{self, ConfigError},
        blob_store::blob_store::{
            self, BlobError, BlobReader, BlobWriter, HostBlobReader, HostBlobWriter,
        },
        data_layer::store::{
            self, CollectionSchema, DataLayerError, Mutation, QueryOptions, QueryResult,
            RecordReadValue, RecordWriteValue,
        },
        host::{context, context::Host},
        messaging::host_api::{self, MessagingError},
        vault::vault::{self, VaultError},
    },
};
use tokio::{fs as tokio_fs, sync::oneshot, time};
use tracing::{debug, error, info, warn};
use wasmtime::{
    Config, Engine, InstanceAllocationStrategy, PoolingAllocationConfig, Store, StoreLimits,
    StoreLimitsBuilder, Trap,
    component::{
        Component, Func, HasSelf, Instance, InstancePre, Linker, Resource, Val,
        types::ComponentItem,
    },
};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxView, WasiView, p2};
use zeroize::Zeroizing;

use crate::conversions::{json_to_wasm_params, wasm_results_to_json_string};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmResourceQuota {
    pub max_instructions: Option<u64>,
    pub max_memory_bytes: Option<u64>,
}

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
        BrokerMessagingError::PermissionDenied => MessagingError::PermissionDenied,
        BrokerMessagingError::Internal(msg) => MessagingError::Internal(msg),
    }
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
    pub is_init_context: bool,
    pub config_generation: u64,
    pub messaging: MessagingContext,
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
        is_init_context: bool,
        config_generation: u64,
        messaging: MessagingContext,
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
            is_init_context,
            config_generation,
            messaging,
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
        let namespaced = namespace_topic(&self.component_id, &topic);
        let broker = self.messaging.broker.clone();
        broker.publish(&namespaced, payload).await.map_err(map_broker_error)
    }

    async fn subscribe(&mut self, topic: String) -> Result<(), MessagingError> {
        let namespaced = namespace_topic(&self.component_id, &topic);
        let service_id = self.component_id.clone();
        let storage_provider = self.storage_provider.clone();
        let engine = self.messaging.engine.clone();

        storage_provider
            .save_messaging_subscription(&service_id, &namespaced)
            .await
            .map_err(|e| MessagingError::Internal(e.to_string()))?;

        let Some(engine) = engine.upgrade() else {
            return Err(MessagingError::Internal(
                "sandbox engine unavailable for subscription registration".to_string(),
            ));
        };
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

        if let Some(engine) = engine.upgrade() {
            engine.subscriptions.remove(&(service_id, namespaced));
        }
        Ok(())
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
        // TODO(M4): replace is_init_context with Admin UCAN check
        if !self.is_init_context {
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

/// Engine: Passive code module that wraps low-level OS operations
/// to spin up Wasmtime or Podman instances.
pub struct AppSandboxEngine {
    blobs_dir: PathBuf,
    engine: Engine,
    linker: Linker<HostState>,
    // Cache of pre-linked instances for fast instantiation
    components: DashMap<String, (InstancePre<HostState>, Option<WasmResourceQuota>)>,
    default_max_instructions: Option<u64>,
    default_max_memory_bytes: Option<u64>,
    _shutdown_tx: Option<oneshot::Sender<()>>,
    pub key_store: Arc<KeyStore>,
    pub storage_provider: Arc<dyn StorageProvider>,
    pub blob_provider: Arc<dyn BlobProvider>,
    pub messaging_broker: Arc<MqttBroker>,
    /// Set once, immediately after the engine is wrapped in an `Arc` by its
    /// owner (see module docs on [`MessagingContext`]). Lets a live
    /// `subscribe()` call's forwarding task reach back into the engine to
    /// invoke `deliver_message` long after the `Store` that made the call
    /// is gone.
    pub self_weak: OnceLock<Weak<AppSandboxEngine>>,
    /// Live guest-delivery subscriptions, keyed `(service_id,
    /// namespaced_topic)`. Dropping an entry unsubscribes from the broker
    /// (see `SubscriptionHandle::drop`).
    subscriptions: DashMap<(String, String), SubscriptionHandle>,
}

impl Debug for AppSandboxEngine {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppSandboxEngine")
            .field("blobs_dir", &self.blobs_dir)
            .field("components_len", &self.components.len())
            .finish()
    }
}

impl AppSandboxEngine {
    /// Helper to validate service ID against path traversal and invalid
    /// characters
    pub fn validate_service_id(service_id: &str) -> Result<()> {
        if service_id.is_empty()
            || service_id.contains('/')
            || service_id.contains('\\')
            || service_id.contains("..")
            || Path::new(service_id).is_absolute()
        {
            return Err(anyhow::anyhow!(
                "Invalid service_id: path traversal or invalid characters"
            ));
        }
        Ok(())
    }

    /// Initializes the App Sandbox and warms up any existing WASM endpoints
    #[allow(clippy::too_many_arguments)]
    pub async fn init(
        config: &SubstrateConfig,
        endpoints: Vec<(String, String, SubstrateEndpoint)>,
        key_store: Arc<KeyStore>,
        storage_provider: Arc<dyn StorageProvider>,
        blob_provider: Arc<dyn BlobProvider>,
        messaging_broker: Arc<MqttBroker>,
    ) -> anyhow::Result<Self> {
        let component_dir = config.storage.blobs_dir.join("app_sandbox");

        // Ensure blobs directory exists
        if !component_dir.exists() {
            tokio_fs::create_dir_all(&component_dir).await?;
        }

        // Read these limits from `config` based on the hardware tier
        let (max_instances, max_memory) = if let Some(sandbox_config) = &config.roles.app_sandbox {
            (sandbox_config.max_concurrent_instances, sandbox_config.memory_limit_bytes() as usize)
        } else {
            (10, 128 * 1024 * 1024)
        };

        let engine = Self::build_wasm_engine(Some(max_instances), Some(max_memory))?;
        let linker = Self::build_wasm_linker(&engine)?;

        // Component cache
        let components = DashMap::new();

        let (default_max_instructions, default_max_memory_bytes) =
            if let Some(sandbox_config) = &config.roles.app_sandbox {
                (sandbox_config.default_max_instructions, sandbox_config.default_max_memory_bytes)
            } else {
                (Some(10_000_000_000), Some(256 * 1024 * 1024))
            };

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

        let app_engine = Self {
            blobs_dir: component_dir,
            engine,
            linker,
            components,
            default_max_instructions,
            default_max_memory_bytes,
            _shutdown_tx: Some(shutdown_tx),
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            self_weak: OnceLock::new(),
            subscriptions: DashMap::new(),
        };

        for (service_id, _interface_name, endpoint) in endpoints {
            if let SubstrateEndpoint::WasmChannel { service_id: channel_id } = endpoint {
                info!(
                    service_id = %service_id,
                    channel_id = %channel_id,
                    "Warming up WASM component"
                );

                if let Err(e) = app_engine.load_cached_wasm(&service_id).await {
                    error!("Failed to warm up WASM component {}: {}", service_id, e);
                }
            }
        }

        let engine_clone = app_engine.engine.clone();
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_millis(100));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        engine_clone.increment_epoch();
                    }
                    _ = &mut shutdown_rx => {
                        break;
                    }
                }
            }
        });

        Ok(app_engine)
    }

    /// Helper to build the Wasmtime Engine
    pub fn build_wasm_engine(
        max_instances: Option<u32>,
        max_memory: Option<usize>,
    ) -> Result<Engine> {
        let mut wasmtime_config = Config::new();
        wasmtime_config.wasm_component_model(true);
        wasmtime_config.consume_fuel(true);
        wasmtime_config.epoch_interruption(true);

        if let (Some(instances), Some(memory)) = (max_instances, max_memory) {
            wasmtime_config.memory_init_cow(true);
            let mut pooling_config = PoolingAllocationConfig::default();
            pooling_config.total_component_instances(instances);
            pooling_config.max_memory_size(memory);
            wasmtime_config
                .allocation_strategy(InstanceAllocationStrategy::Pooling(pooling_config));
        }

        Engine::new(&wasmtime_config).map_err(Into::into)
    }

    /// Helper to build the Wasmtime Linker
    pub fn build_wasm_linker(engine: &Engine) -> Result<Linker<HostState>> {
        let mut linker = Linker::new(engine);
        p2::add_to_linker_async(&mut linker)?;
        context::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        vault::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        store::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        app_config::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        blob_store::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        host_api::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        Ok(linker)
    }

    /// Helper to fetch WASM bytes from a source
    async fn fetch_wasm_bytes(source: &ArtifactSource) -> Result<Vec<u8>> {
        match source {
            ArtifactSource::Url(url) => {
                info!("Fetching WASM from URL: {}", url);
                Ok(reqwest::get(url)
                    .await
                    .context("Failed to fetch WASM from URL")?
                    .bytes()
                    .await
                    .context("Failed to read WASM bytes")?
                    .to_vec())
            }
            ArtifactSource::Binary(b) => Ok(b.clone()),
        }
    }

    /// Helper to verify the hash of WASM bytes
    fn verify_wasm_hash(bytes: &[u8], expected_hash: Option<&str>) -> Result<()> {
        if let Some(expected_hash) = expected_hash {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            let computed_hash = hex::encode(hasher.finalize());

            let expected_hash_clean =
                expected_hash.strip_prefix("sha256:").unwrap_or(expected_hash);

            if computed_hash != expected_hash_clean {
                return Err(anyhow::anyhow!(
                    "Hash mismatch: expected {expected_hash_clean}, got {computed_hash}"
                ));
            }
            info!("WASM hash verified successfully");
        }
        Ok(())
    }

    /// Helper to extract a WASM function and its result length. When
    /// `interface_name` is `Some`, looks up `method_name` nested inside that
    /// named interface's exported instance (the shape of ordinary `interface`
    /// exports). When `None`, looks up `method_name` directly as a root-level
    /// component export -- the shape of a WIT world's own `export foo: func`
    /// declarations, such as the `data-layer-guest` world's `init`/`migrate`.
    pub fn get_wasm_func(
        store: &mut Store<HostState>,
        instance: &Instance,
        interface_name: Option<&str>,
        method_name: &str,
    ) -> Result<(Func, usize, ComponentItem)> {
        let (item, func_idx) = match interface_name {
            Some(interface_name) => {
                let (_, instance_idx) = instance
                    .get_export(&mut *store, None, interface_name)
                    .ok_or_else(|| anyhow::anyhow!("Interface '{interface_name}' not found"))?;
                instance.get_export(&mut *store, Some(&instance_idx), method_name).ok_or_else(
                    || {
                        anyhow::anyhow!(
                            "Method '{method_name}' not found in interface '{interface_name}'"
                        )
                    },
                )?
            }
            None => instance
                .get_export(&mut *store, None, method_name)
                .ok_or_else(|| anyhow::anyhow!("Root export '{method_name}' not found"))?,
        };

        let func = instance
            .get_func(&mut *store, func_idx)
            .ok_or_else(|| anyhow::anyhow!("Method is not a function"))?;

        let results_len = match &item {
            ComponentItem::ComponentFunc(f) => f.results().len(),
            _ => 0,
        };

        Ok((func, results_len, item))
    }

    /// Deploy and compile a WASM component for a given service
    pub async fn deploy_wasm(&self, service_id: &str, manifest: &DeployManifest) -> Result<()> {
        Self::validate_service_id(service_id)?;
        info!("AppSandboxEngine: Deploying Wasm component for {}", service_id);

        let ServiceType::Wasm(wasm_manifest) = &manifest.service_type else {
            return Err(anyhow::anyhow!("Expected Wasm manifest"));
        };

        // 1. Fetch bytes
        let bytes = Self::fetch_wasm_bytes(&wasm_manifest.source).await?;

        // 2. Verify hash
        Self::verify_wasm_hash(&bytes, wasm_manifest.hash.as_deref())?;

        // 3. Store locally in blobs_dir
        let file_path = self.blobs_dir.join(format!("{service_id}.wasm"));
        tokio_fs::write(&file_path, &bytes).await.context("Failed to save WASM binary locally")?;

        info!("WASM binary stored at {:?}", file_path);

        let quota = manifest.config.quota.as_ref().map(|q| WasmResourceQuota {
            max_instructions: q.max_instructions,
            max_memory_bytes: q.max_memory_bytes,
        });

        if let Some(ref q) = quota {
            let quota_path = self.blobs_dir.join(format!("{service_id}.quota.json"));
            if let Ok(quota_json) = serde_json::to_string(q) {
                let _ = tokio_fs::write(&quota_path, quota_json).await;
            }
        }

        // 4. Compile and cache the component; drop the raw bytes immediately to free
        //    memory
        self.compile_and_cache_wasm(service_id, &bytes, quota)?;
        drop(bytes);

        // 5. Invoke the guest's schema lifecycle hook: `init()` on a fresh service (no
        //    existing database), `migrate()` on a re-deploy of a service with existing
        //    state. Checked here, before anything else can lazily open the service DB
        //    and thereby create it.
        let is_first_deploy = !self
            .storage_provider
            .service_exists(service_id)
            .await
            .context("failed to check for pre-existing service state")?;
        let hook = if is_first_deploy {
            "init"
        } else {
            // TODO(M5): full snapshot/rollback safety net for migrate() is
            // deferred to M5 [LFC-VER]. migrate() may execute destructive
            // DDL; there is no automatic rollback on partial failure in M3A.
            "migrate"
        };
        self.invoke_lifecycle_hook(service_id, hook)
            .await
            .with_context(|| format!("{hook}() lifecycle hook failed for service {service_id}"))?;

        Ok(())
    }

    /// Execute a WASM component for a given service
    pub async fn execute_wasm(
        &self,
        service_id: &str,
        interface_name: &str,
        request: &JsonRpcRequest,
    ) -> Result<String> {
        Self::validate_service_id(service_id)?;
        struct ActiveInstanceGuard;
        impl ActiveInstanceGuard {
            fn new() -> Self {
                metrics::gauge!("substrate.wasm.active_instances").increment(1.0);
                Self
            }
        }
        impl Drop for ActiveInstanceGuard {
            fn drop(&mut self) {
                metrics::gauge!("substrate.wasm.active_instances").decrement(1.0);
            }
        }

        let _guard = ActiveInstanceGuard::new();
        debug!("starting to execute wasm");

        // TODO: Later optimize this by caching things like function parameter details
        // on first execution, so we don't have to do the same lookups every time.
        let (mut store, func, results_len, item) =
            self.prepare_wasm_execution(service_id, interface_name, &request.method).await?;

        // Parse parameters based on ComponentFunc signature
        let params_iter = match &item {
            ComponentItem::ComponentFunc(f) => f.params(),
            _ => return Err(anyhow::anyhow!("Expected a function item")),
        };

        debug!("extracted the function and parameter iter");

        // Dynamic parameter resolution
        let json_params = match &request.params {
            Value::Array(arr) => arr.clone(),
            other => vec![other.clone()],
        };

        let wasm_params = json_to_wasm_params(params_iter, json_params)?;

        debug!("created input types");

        let mut wasm_results = vec![Val::Bool(false); results_len];
        debug!("created result types");

        let exec_start = Instant::now();
        let res = func.call_async(&mut store, &wasm_params, &mut wasm_results).await;
        metrics::histogram!("substrate.wasm.execution_ms")
            .record(exec_start.elapsed().as_secs_f64() * 1000.0);

        debug!("called wasm function, processing results");

        if let Err(e) = res {
            if let Some(Trap::OutOfFuel) = e.downcast_ref::<Trap>() {
                warn!("Wasm execution exceeded fuel limit for service: {}", service_id);
                return Err(anyhow::anyhow!("QuotaExceeded: Wasm execution exceeded fuel limit"));
            }
            let err_str = e.root_cause().to_string();
            if err_str.contains("all fuel consumed") || err_str.contains("out of fuel") {
                warn!("Wasm execution exceeded fuel limit for service: {}", service_id);
                return Err(anyhow::anyhow!("QuotaExceeded: Wasm execution exceeded fuel limit"));
            }
            if err_str.contains("exceeded its memory limits") || err_str.contains("MemoryFault") {
                return Err(anyhow::anyhow!("MemoryFault: Wasm execution exceeded memory limit"));
            }
            return Err(e.into());
        }

        wasm_results_to_json_string(&wasm_results)
    }

    /// Helper shared by `prepare_wasm_execution` and `invoke_lifecycle_hook`:
    /// looks up the pre-linked component, resolves its resource quotas,
    /// builds a fresh `HostState`/`Store`, and instantiates it.
    async fn build_store_and_instantiate(
        &self,
        service_id: &str,
        is_init_context: bool,
    ) -> Result<(Store<HostState>, Instance)> {
        // Look up the pre-linked component instance
        let (instance_pre, quota) = {
            let entry = self
                .components
                .get(service_id)
                .ok_or_else(|| anyhow::anyhow!("Component not found for service {service_id}"))?;
            entry.value().clone()
        };
        debug!("looked up pre-linked component");

        // Resolve quotas
        let max_instructions =
            quota.as_ref().and_then(|q| q.max_instructions).or(self.default_max_instructions);

        let max_memory_bytes = quota
            .as_ref()
            .and_then(|q| q.max_memory_bytes)
            .or(self.default_max_memory_bytes)
            .map(|m| m as usize);

        let config_generation =
            match self.storage_provider.get_latest_config_generation(service_id).await {
                Ok(Some((g, _))) => g,
                Ok(None) => 0,
                Err(e) => {
                    error!("Failed to fetch config generation for {}: {}", service_id, e);
                    0
                }
            };

        // Create host state
        let messaging = MessagingContext {
            broker: self.messaging_broker.clone(),
            engine: self.self_weak.get().cloned().unwrap_or_default(),
        };
        let host_state = HostState::new(
            service_id.to_string(),
            max_memory_bytes,
            self.key_store.clone(),
            self.storage_provider.clone(),
            self.blob_provider.clone(),
            is_init_context,
            config_generation,
            messaging,
        );

        debug!("created wasi ctx and host state");

        // Create a new store
        let mut store = Store::new(&self.engine, host_state);

        store.limiter(|state| state);
        store.epoch_deadline_trap();
        store.set_epoch_deadline(50); // 50 * 100ms = 5 seconds wall-clock timeout

        if let Some(instructions) = max_instructions {
            store.set_fuel(instructions)?;
        }

        let inst_start = Instant::now();
        let instance = instance_pre.instantiate_async(&mut store).await?;
        metrics::histogram!("substrate.wasm.instantiation_ms")
            .record(inst_start.elapsed().as_secs_f64() * 1000.0);

        debug!("instantiated store and instance");

        Ok((store, instance))
    }

    /// Helper to prepare WASM execution context and extract function
    async fn prepare_wasm_execution(
        &self,
        service_id: &str,
        interface_name: &str,
        method_name: &str,
    ) -> Result<(Store<HostState>, Func, usize, ComponentItem)> {
        let is_init_context = method_name == "init" || method_name == "migrate";
        let (mut store, instance) =
            self.build_store_and_instantiate(service_id, is_init_context).await?;

        // Use the helper to extract the function
        let (func, results_len, item) =
            Self::get_wasm_func(&mut store, &instance, Some(interface_name), method_name)?;

        debug!("extracted the interface and method export indices");

        Ok((store, func, results_len, item))
    }

    /// Invokes a guest lifecycle export (`init` or `migrate`) declared
    /// directly on the `data-layer-guest` world, if the deployed component
    /// exports it. Components that don't declare the export (e.g. a plain
    /// component with no data-layer usage, like the `greeter` test
    /// component) are left untouched -- this makes it safe to call
    /// unconditionally on every deploy.
    async fn invoke_lifecycle_hook(&self, service_id: &str, hook: &str) -> Result<()> {
        let (mut store, instance) = self.build_store_and_instantiate(service_id, true).await?;

        if instance.get_export(&mut store, None, hook).is_none() {
            debug!(service_id, hook, "component does not export lifecycle hook, skipping");
            return Ok(());
        }

        let (func, results_len, _item) = Self::get_wasm_func(&mut store, &instance, None, hook)?;
        let mut results = vec![Val::Bool(false); results_len];
        func.call_async(&mut store, &[], &mut results).await?;

        if let Some(Val::Result(Err(Some(boxed)))) = results.first()
            && let Val::String(msg) = boxed.as_ref()
        {
            return Err(anyhow::anyhow!("{hook}() failed: {msg}"));
        }
        Ok(())
    }

    /// Core in-memory subscribe logic shared by a live guest `subscribe()`
    /// call and substrate-startup replay (the latter has no `HostState` to
    /// call through, since it runs before any request is served). Spawns a
    /// forwarding task that calls `deliver_message` per broker message and
    /// exits when the broker's receiver closes (including when this
    /// engine itself is dropped, via `MqttBroker`'s `CancellationToken`).
    pub async fn register_internal_subscription(
        &self,
        service_id: &str,
        namespaced_topic: &str,
    ) -> Result<()> {
        let (handle, mut receiver) = self
            .messaging_broker
            .subscribe(namespaced_topic)
            .await
            .map_err(|e| anyhow::anyhow!("broker subscribe failed: {e}"))?;

        let engine_weak = self.self_weak.get().cloned().unwrap_or_default();
        let service_id_owned = service_id.to_string();
        tokio::spawn(async move {
            while let Some((topic, payload)) = receiver.recv().await {
                let Some(engine) = engine_weak.upgrade() else { break };
                engine.deliver_message(&service_id_owned, &topic, payload).await;
            }
        });

        self.subscriptions.insert((service_id.to_string(), namespaced_topic.to_string()), handle);
        Ok(())
    }

    /// Drops every live guest-delivery subscription for `service_id`
    /// (called from `ControlPlaneService::undeploy`'s cleanup).
    pub fn unsubscribe_all(&self, service_id: &str) {
        self.subscriptions.retain(|(sid, _topic), _handle| sid != service_id);
    }

    /// Invokes the deployed component's exported `guest-api::handle-message`
    /// with a freshly-instantiated `Store` (same reasoning as any other
    /// invocation -- see `build_store_and_instantiate`), if it declares
    /// that export. If not, the message is silently discarded (per
    /// ADR-0010): this makes it safe to call for every subscription
    /// regardless of whether the target component implements messaging.
    async fn deliver_message(&self, service_id: &str, topic: &str, payload: Vec<u8>) {
        let (mut store, instance) = match self.build_store_and_instantiate(service_id, false).await
        {
            Ok(pair) => pair,
            Err(e) => {
                debug!(
                    service_id,
                    error = %e,
                    "messaging: failed to instantiate component for delivery"
                );
                return;
            }
        };

        const GUEST_API_INTERFACE: &str = "syneroym:messaging/guest-api@0.1.0";
        let (func, results_len, _item) = match Self::get_wasm_func(
            &mut store,
            &instance,
            Some(GUEST_API_INTERFACE),
            "handle-message",
        ) {
            Ok(found) => found,
            Err(_) => {
                debug!(
                    service_id,
                    "messaging: component does not export guest-api::handle-message, discarding"
                );
                return;
            }
        };

        let args =
            [Val::String(topic.to_string()), Val::List(payload.into_iter().map(Val::U8).collect())];
        let mut results = vec![Val::Bool(false); results_len];
        if let Err(e) = func.call_async(&mut store, &args, &mut results).await {
            warn!(service_id, error = %e, "messaging: handle-message invocation trapped");
            return;
        }

        if let Some(Val::Result(Err(Some(boxed)))) = results.first()
            && let Val::String(msg) = boxed.as_ref()
        {
            warn!(service_id, error = %msg, "messaging: handle-message returned an error");
        }
    }

    /// Simple test function to invoke test context
    pub async fn invoke_test_context(
        &self,
        service_id: &str,
        component_id: &str,
        request_ctx: &str,
    ) -> Result<String> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "run".to_string(), // Default method for test
            params: Value::String(request_ctx.to_string()),
            id: None,
        };
        self.execute_wasm(service_id, component_id, &request).await
    }

    /// Stop and evict a running Wasm component from the in-memory cache.
    pub async fn stop_wasm(&self, service_id: &str) -> Result<()> {
        Self::validate_service_id(service_id)?;
        info!(service_id = %service_id, "AppSandboxEngine: stopping Wasm component");
        self.components.remove(service_id);
        metrics::gauge!("substrate.wasm.component_cache_size").set(self.components.len() as f64);
        Ok(())
    }

    /// Remove a stopped Wasm component's binary from disk.
    pub async fn remove_wasm(&self, service_id: &str) -> Result<()> {
        Self::validate_service_id(service_id)?;
        info!(service_id = %service_id, "AppSandboxEngine: removing Wasm component");
        let file_path = self.blobs_dir.join(format!("{service_id}.wasm"));
        if file_path.exists() {
            tokio_fs::remove_file(&file_path)
                .await
                .with_context(|| format!("Failed to remove WASM file {file_path:?}"))?;
        }
        let quota_path = self.blobs_dir.join(format!("{service_id}.quota.json"));
        if quota_path.exists() {
            let _ = tokio_fs::remove_file(&quota_path).await;
        }
        Ok(())
    }

    /// Helper to load a cached WASM component from disk and compile it
    async fn load_cached_wasm(&self, service_id: &str) -> Result<()> {
        Self::validate_service_id(service_id)?;
        let file_path = self.blobs_dir.join(format!("{service_id}.wasm"));
        if file_path.exists() {
            let bytes = tokio_fs::read(&file_path)
                .await
                .context(format!("Failed to read WASM file {file_path:?}"))?;
            let quota_path = self.blobs_dir.join(format!("{service_id}.quota.json"));
            let quota = if quota_path.exists() {
                if let Ok(quota_json) = tokio_fs::read_to_string(&quota_path).await {
                    serde_json::from_str::<WasmResourceQuota>(&quota_json).ok()
                } else {
                    None
                }
            } else {
                None
            };
            self.compile_and_cache_wasm(service_id, &bytes, quota)?;
        } else {
            warn!("WASM file not found on disk for service: {:?}", file_path);
        }
        Ok(())
    }

    /// Helper to compile a WASM binary and store it in the cache
    pub fn compile_and_cache_wasm(
        &self,
        service_id: &str,
        bytes: &[u8],
        quota: Option<WasmResourceQuota>,
    ) -> Result<()> {
        let component = Component::new(&self.engine, bytes)
            .map_err(|e| anyhow::anyhow!("Failed to compile WASM component: {e}"))?;

        let instance_pre = self
            .linker
            .instantiate_pre(&component)
            .map_err(|e| anyhow::anyhow!("Failed to pre-link WASM component: {e}"))?;

        self.components.insert(service_id.to_string(), (instance_pre, quota));
        info!("WASM component compiled and cached for {}", service_id);
        metrics::gauge!("substrate.wasm.component_cache_size").set(self.components.len() as f64);
        Ok(())
    }

    /// Spin up a new Podman instance
    pub async fn deploy_podman(&self, _service_id: &str, _manifest: &[u8]) -> Result<()> {
        info!("AppSandboxEngine: Deploying Podman container for {}", _service_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{env, fs};

    use syneroym_core::test_constants;
    use syneroym_data_blob::ObjectStoreBlobProvider;
    use syneroym_data_db::SqliteStorageProvider;
    use syneroym_mqtt_broker::MqttBrokerConfig;
    use wasmtime::component::Component;

    use super::*;

    /// Test-only blob provider: in-memory backend, effectively unlimited
    /// quota.
    fn test_blob_provider() -> Arc<dyn BlobProvider> {
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None))
    }

    /// Test-only messaging context: a real (but throwaway, no network
    /// listener) broker with no engine backreference -- sufficient for
    /// tests that don't exercise guest-delivery messaging.
    fn test_messaging_context() -> MessagingContext {
        MessagingContext {
            broker: Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
            engine: Weak::new(),
        }
    }

    #[tokio::test]
    async fn test_list_interfaces() {
        let engine = AppSandboxEngine::build_wasm_engine(None, None).unwrap();
        let linker = AppSandboxEngine::build_wasm_linker(&engine).unwrap();

        let key_store = Arc::new(KeyStore::new());
        let storage_provider = Arc::new(
            SqliteStorageProvider::new(tempfile::tempdir().unwrap().path(), false).unwrap(),
        );
        let host_state = HostState::new(
            "test_component".to_string(),
            None,
            key_store,
            storage_provider,
            test_blob_provider(),
            false,
            0,
            test_messaging_context(),
        );

        let mut store = Store::new(&engine, host_state);

        let component_path = test_constants::greeter_wasm_path();
        let wasm_bytes = if let Ok(bytes) = fs::read(&component_path) {
            bytes
        } else {
            println!(
                "Skipping test_list_interfaces: WASM artifact not found at {}",
                component_path.display()
            );
            return;
        };

        let component: Component =
            Component::new(&engine, &wasm_bytes).expect("Failed to compile WASM component");
        for interface in component.component_type().exports(&engine) {
            println!("Listing interface: {interface:?}");
        }

        match linker.instantiate_async(&mut store, &component).await {
            Ok(instance) => {
                let interface_name = test_constants::GREETER_INTERFACE_NAME;
                let method_name = "greet";

                // Use the helper function to extract function and result size
                match AppSandboxEngine::get_wasm_func(
                    &mut store,
                    &instance,
                    Some(interface_name),
                    method_name,
                ) {
                    Ok((func, results_len, _item)) => {
                        println!("Function export: {func:?}");
                        let mut wasm_results = vec![Val::Bool(false); results_len];

                        let result = func
                            .call_async(
                                &mut store,
                                &[Val::String("TestUser".to_string())],
                                &mut wasm_results,
                            )
                            .await
                            .map_err(|e| anyhow::anyhow!("Failed to call function: {e}"));
                        println!("Function call result: {result:?} is {wasm_results:?}");
                    }
                    Err(e) => {
                        println!("Failed to get wasm func: {e}");
                    }
                }
            }
            Err(err) => {
                println!("Error instantiating component: {err}");
            }
        }
    }

    #[tokio::test]
    async fn test_wasm_quotas() {
        let wat = r#"
(component
  (core module $m
    (func (export "loop_forever")
      (loop $l
        br $l
      )
    )
    (func (export "allocate_too_much") (param $pages i32) (result i32)
      (memory.grow (local.get $pages))
    )
    (memory (export "memory") 1)
  )
  (core instance $i (instantiate $m))
  (func $loop_forever (canon lift (core func $i "loop_forever")))
  (func $allocate_too_much (param "pages" u32) (result s32) (canon lift (core func $i "allocate_too_much")))
  (instance $interface
    (export "loop-forever" (func $loop_forever))
    (export "allocate-too-much" (func $allocate_too_much))
  )
  (export "test-interface" (instance $interface))
)
"#;
        let engine =
            AppSandboxEngine::build_wasm_engine(Some(10), Some(128 * 1024 * 1024)).unwrap();
        let linker = AppSandboxEngine::build_wasm_linker(&engine).unwrap();

        let app_engine = AppSandboxEngine {
            blobs_dir: env::temp_dir(),
            engine,
            linker,
            components: DashMap::new(),
            default_max_instructions: Some(10_000),
            default_max_memory_bytes: Some(1024 * 1024), // 1MB
            _shutdown_tx: None,
            key_store: Arc::new(KeyStore::new()),
            storage_provider: Arc::new(SqliteStorageProvider::new(env::temp_dir(), false).unwrap()),
            blob_provider: test_blob_provider(),
            messaging_broker: Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
            self_weak: OnceLock::new(),
            subscriptions: DashMap::new(),
        };

        // Cache the test component
        app_engine.compile_and_cache_wasm("test_service", wat.as_bytes(), None).unwrap();

        // 1. Test infinite loop (fuel limit)
        let request_loop = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "loop-forever".to_string(),
            params: Value::Array(vec![]),
            id: None,
        };
        let res_loop =
            app_engine.execute_wasm("test_service", "test-interface", &request_loop).await;
        assert!(res_loop.is_err());
        let err_msg = res_loop.unwrap_err().to_string();
        assert!(err_msg.contains("QuotaExceeded"), "expected QuotaExceeded, got: {}", err_msg);

        // 2. Test memory allocation limit
        // 1 page is 64KB. We try to allocate 100 pages (6.4MB), which exceeds the 1MB
        // limit.
        let request_mem = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "allocate-too-much".to_string(),
            params: Value::Array(vec![Value::Number(serde_json::Number::from(100))]),
            id: None,
        };
        let res_mem = app_engine.execute_wasm("test_service", "test-interface", &request_mem).await;
        assert!(res_mem.is_err());
        let err_msg = res_mem.unwrap_err().to_string();
        assert!(
            err_msg.contains("MemoryFault") || err_msg.contains("failed to grow memory"),
            "expected MemoryFault or failed to grow memory, got: {}",
            err_msg
        );
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
            false,
            generation,
            test_messaging_context(),
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
            false,
            gen2_a,
            test_messaging_context(),
        );
        let mut host_b = HostState::new(
            "svc_b".to_string(),
            None,
            Arc::new(KeyStore::new()),
            storage.clone(),
            test_blob_provider(),
            false,
            gen1_b,
            test_messaging_context(),
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
            false,
            gen1_a,
            test_messaging_context(),
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
            false,
            0,
            test_messaging_context(),
        );

        let result = vault::Host::reveal(&mut host_state, "does-not-exist".to_string()).await;
        assert!(matches!(result, Err(VaultError::NotFound)));
    }
}
