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
    mem,
    sync::{Arc, Weak},
    time::Duration,
};

use serde_json::Value;
use syneroym_app_orchestration::{AppInstanceId, LogicalResolver, LogicalServiceName, TopologyKey};
use syneroym_core::local_registry::SubstrateEndpoint;
use syneroym_data_blob::{
    BlobError as BlobStoreError, HostDownloadSession, HostUploadSession, traits::BlobProvider,
};
use syneroym_data_db::{
    QueryAuth, auth,
    traits::{ServiceStore, StorageProvider},
};
use syneroym_data_keystore::KeyStore;
use syneroym_fdae::{Mode, Policy};
use syneroym_mqtt_broker::{
    MessagingError as BrokerMessagingError, MqttBroker, namespace_topic,
    namespace_topic_for_publish,
};
use syneroym_rpc::{
    AbacError, Ability, AuthLevel, CallOrigin, CallerContext, CandidateRow,
    ProxyError as RpcProxyError, ProxyProtocol, ProxyRequest, QueuedCall, QueuedTarget,
    ResourceUri, RowAuthorizer, SagaBegin, SagaState as RpcSagaState, SagaStepRequest,
    ServiceProxy, apply_stage4, union_masked_fields,
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
    proxy::{
        proxy::{self, CallOptions, CallTarget, CalleeError},
        saga::{self, SagaState as WitSagaState, SagaStatus},
    },
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
    /// Compiled FDAE policy for this service, or `None` if policy-absent
    /// (today's unfiltered behavior). Loading a real policy at instantiation
    /// is Phase 4; Phase 3 only threads the field through.
    pub fdae_policy: Option<Arc<Policy>>,
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
    /// Stage-4 after-step instances (`AuthLevel::LocalReadOnly`) get this
    /// set: every mutating and egress host function hard-denies. Not
    /// derivable from `caller.auth` alone -- write host paths carry no
    /// capability gate today (D-04-02-f), so the check has to live somewhere
    /// that isn't the capability layer.
    pub read_only: bool,
    /// Weak handle to the after-step invoker (ADR-0017 §7). `Weak`, not
    /// `Arc`: the only implementation is `AppSandboxEngine`, which owns this
    /// state's `Store` -- same cycle reasoning as `service_proxy`.
    pub row_authorizer: Weak<dyn RowAuthorizer>,
    /// The app instance this component was deployed as part of, from the
    /// substrate's own records -- never from the guest (ADR-0021 §2: a
    /// guest that could name an app instance could address an arbitrary
    /// one). `None` for a standalone deploy, which resolves no dependency.
    pub app_instance_id: Option<String>,
    /// Resolves a declared dependency name to a member's master DID.
    /// `Arc`, unlike `service_proxy`: `LogicalResolver` holds only an
    /// `Arc<dyn AppRegistry>` and no path back to the engine, so there is
    /// no cycle to guard against.
    pub logical_resolver: Arc<LogicalResolver>,
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
        fdae_policy: Option<Arc<Policy>>,
        read_only: bool,
        row_authorizer: Weak<dyn RowAuthorizer>,
        app_instance_id: Option<String>,
        logical_resolver: Arc<LogicalResolver>,
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
            fdae_policy,
            config_generation,
            messaging,
            streaming,
            service_proxy,
            read_only,
            row_authorizer,
            app_instance_id,
            logical_resolver,
        }
    }

    /// Builds the `QueryAuth` for the current request from `fdae_policy` +
    /// `caller.session`, or `None` on the policy-absent path (today's
    /// unfiltered behavior). Slice B3 Phase 4: runs `syneroym_fdae::
    /// plan_read` itself (rather than letting `data_db` call the
    /// local-only `compile_read` internally), and when the policy's
    /// selected paths need a remote relationship fetch (pipeline stage 2),
    /// resolves it via `syneroym_rpc::resolve_fetches` + `syneroym_fdae::
    /// finalize` before ever reaching the store -- `QueryAuth.resolved_sieve`
    /// carries the result through.
    ///
    /// **Fails closed on a fetch error** (timeout, transport error, an
    /// unverifiable/expired `RelationshipProof`): mapped to
    /// `DataLayerError::PermissionDenied` here, exactly the fail-closed
    /// shape `data_db`'s own watchdog/compile-error paths already use for
    /// Mode B. `check_access`'s own call site further maps that to `Ok(false)`
    /// (Mode A's convention, matching a `PolicyError` compile failure).
    ///
    /// **`AuthLevel::LocalElevated` is exempt.** This is not the
    /// `AuthLevel::System` carve-out `synsvc_native.rs::query_auth`
    /// deliberately refuses (that one would let a guest's self-proxy escape
    /// its own policy) -- `LocalElevated` is a categorically different,
    /// host-synthesized-only context: `engine.rs`'s `invoke_lifecycle_hook`
    /// is the sole producer, for `init`/`migrate`, and no guest input can
    /// ever request it. Its capability (`data-layer/admin` on the service's
    /// own resource) already entails `data-layer/read` and covers every
    /// collection, so a policy with a `caller`-terminal permission compiles
    /// a *real* (non-`deny_all`) sieve here -- one bound to
    /// `"system:local-elevated:<service_id>"`, a DID no principal row will
    /// ever hold, so it silently returns zero rows rather than failing. A
    /// migration that reads its own data to decide how to rewrite it would
    /// see nothing and could act on that emptiness. Sieving this context
    /// was never the intent -- `execute-ddl`/`query-raw`'s own admin gate
    /// exists specifically so lifecycle hooks act with full authority over
    /// their own service's data.
    ///
    /// **`AuthLevel::LocalReadOnly` is exempt too, for a related but
    /// distinct reason (ADR-0017 §7, D-B4-2/D-B4-4).** It is the stage-4
    /// after-step's own identity: the ADR is explicit that the after-step's
    /// optional lookups read this service's data unfiltered -- the service
    /// owner authored the policy and could equally have written the same
    /// call into their service code, and running under the caller's
    /// authority breaks most real policies. Read-only-ness comes from
    /// `HostState.read_only` (hard-denying every mutating/egress host
    /// function), not from the sieve. **This early return is also what
    /// bounds after-step recursion (D-B4-4)**: an after-step instance's own
    /// reads carry no `QueryAuth` at all, hence no sieve, hence no
    /// `abac_permissions` to trigger a second after-step. Narrowing this
    /// exemption without replacing that bound reintroduces unbounded
    /// recursion.
    async fn resolve_query_auth(
        &mut self,
        collection: &str,
        operation: &Ability,
        mode: Mode,
    ) -> Result<Option<QueryAuth<'_>>, DataLayerError> {
        if matches!(self.caller.auth, AuthLevel::LocalElevated | AuthLevel::LocalReadOnly) {
            return Ok(None);
        }
        let Some(policy) = self.fdae_policy.as_ref() else { return Ok(None) };
        // Bound once, up front: `HostState` holds non-`Sync` WASI internals,
        // so a projection like `&self.caller.session` written *after* an
        // `.await` forces the whole `&HostState` receiver into the
        // generator's captured state across that yield point, which breaks
        // the WIT-generated `Host` trait's `Send`-future requirement. Only
        // these two locals (both `Send`, since `Policy`/`SessionContext` are
        // plain `Sync` data) may be read after the await below -- never
        // `self` itself.
        let session = &self.caller.session;
        let service_id = self.component_id.as_str();
        let plan =
            syneroym_fdae::plan_read(policy, collection, session, service_id, operation, mode)
                .map_err(|e| DataLayerError::Internal(e.to_string()))?;
        let resolved_sieve = if plan.fetches.is_empty() {
            plan.local
        } else {
            let proxy = self.service_proxy.upgrade().ok_or_else(|| {
                DataLayerError::Internal(
                    "service proxy unavailable for a cross-service FDAE fetch".to_string(),
                )
            })?;
            // Cloned to an owned value before the `.await` below, for the
            // same `Send`-future reason as above.
            let caller = self.caller.clone();
            let local_service_id = self.component_id.clone();
            let results = syneroym_rpc::resolve_fetches(
                &plan.fetches,
                &caller,
                proxy.as_ref(),
                &local_service_id,
            )
            .await
            .map_err(|e| {
                tracing::warn!(
                    error = %e,
                    collection,
                    "fdae: cross-service relationship fetch failed, denying closed"
                );
                DataLayerError::PermissionDenied
            })?;
            let pending = plan.pending.ok_or_else(|| {
                DataLayerError::Internal(
                    "internal: plan_read reported fetches but no pending sieve".to_string(),
                )
            })?;
            Some(
                syneroym_fdae::finalize(pending, &results)
                    .map_err(|e| DataLayerError::Internal(e.to_string()))?,
            )
        };
        Ok(Some(QueryAuth { policy, session, service_id, resolved_sieve }))
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
        if self.read_only {
            return Err(MessagingError::PermissionDenied);
        }
        let namespaced = namespace_topic_for_publish(&self.component_id, &topic);
        let broker = self.messaging.broker.clone();
        broker.publish(namespaced, payload).await.map_err(map_broker_error)
    }

    async fn subscribe(&mut self, topic: String) -> Result<(), MessagingError> {
        // A subscription registered from a throw-away stage-4 instance
        // would outlive it, and stage-4 is a local, synchronous read-only
        // lookup, not a place to register egress.
        if self.read_only {
            return Err(MessagingError::PermissionDenied);
        }
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
        if self.read_only {
            return Err(MessagingError::PermissionDenied);
        }
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
        if self.read_only {
            return Err(MessagingError::PermissionDenied);
        }
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

/// Applies the host-side CLS field-mask projection to a single read record
/// (ADR-0017 §4, Phase 3). A fail-closed `Err` from `strip_masked_fields`
/// propagates, never a leaked payload.
fn strip_record(
    mut record: RecordReadValue,
    masked_fields: &[String],
) -> Result<RecordReadValue, DataLayerError> {
    record.payload = auth::strip_masked_fields(record.payload, masked_fields)?;
    Ok(record)
}

/// Converts a store-returned record into the stage-4 after-step's candidate
/// shape (ADR-0017 §7) -- the two types share the same fields (both mirror
/// the physical row), so this is a plain field copy.
fn to_candidate_row(record: &RecordReadValue) -> CandidateRow {
    CandidateRow {
        id: record.id.clone(),
        payload: record.payload.clone(),
        creator_id: record.creator_id.clone(),
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

/// The inverse of [`to_candidate_row`] -- reconstructs the WIT record shape
/// from a stage-4-surviving candidate, since `apply_stage4`'s output
/// (`kept`) is no longer positionally aligned with the original row list
/// (denied rows are dropped, not carried as `None`s).
fn from_candidate_row(row: CandidateRow) -> RecordReadValue {
    RecordReadValue {
        id: row.id,
        payload: row.payload,
        creator_id: row.creator_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Maps a stage-4 after-step failure to the `DataLayerError` `get`/`query`
/// actually return (review finding B4-04). Both fail closed either way (no
/// row data reaches the caller), but collapsing every `AbacError` into an
/// empty-but-successful page made "the after-step said no" indistinguishable
/// from "the after-step couldn't run at all" -- and the ADR-0007 "no result
/// is a valid outcome" principle this leaned on covers authorization
/// denials, not infrastructure failures. `Unavailable` (includes the
/// after-step's own pool-exhaustion case, B4-01) and `BudgetExceeded` are
/// resource pressure, reported the same way `data_db`'s watchdog timeout
/// already reports one (`QuotaExceeded`); the rest are the guest's own
/// after-step misbehaving (a missing export, a trap, a malformed or
/// arity-mismatched decision list), reported as `Internal`.
fn map_abac_error(e: AbacError) -> DataLayerError {
    match e {
        AbacError::Unavailable(_)
        | AbacError::BudgetExceeded { .. }
        | AbacError::BatchTooLarge(_)
        | AbacError::PayloadTooLarge { .. } => DataLayerError::QuotaExceeded,
        // `MissingExport`/`ArityMismatch` carry only a service id / row
        // counts -- safe to echo in full, and useful for diagnosing a
        // deploy-time misconfiguration.
        AbacError::MissingExport(_) | AbacError::ArityMismatch { .. } => {
            DataLayerError::Internal(e.to_string())
        }
        // `Trap`/`Malformed` can carry guest-authored (and, for a malformed
        // decision, potentially row-derived) text -- review residual R3:
        // echoing it via `DataLayerError::Internal` puts it on the wire to
        // the calling client, a channel that didn't exist before B4-04's
        // fix (previously an after-step failure never reached the caller
        // at all). A generic message keeps the caller-visible signal to
        // "the after-step failed" without the detail; the detail itself
        // still reaches `AbacTrace::emit`'s (truncated, B4-06) log line,
        // which is the audience it's actually useful to.
        AbacError::Trap { .. } => {
            DataLayerError::Internal("stage-4 after-step trapped".to_string())
        }
        AbacError::Malformed(_) => {
            DataLayerError::Internal("stage-4 after-step returned a malformed decision".to_string())
        }
    }
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
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        store.create_collection(&schema).await
    }

    async fn drop_collection(&mut self, name: String) -> Result<(), DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        // Admin-capability gate, identical to `execute_ddl`'s: dropping an
        // *existing* collection bypasses any per-row policy on it entirely
        // (every row a write-capable-but-otherwise-unreachable caller could
        // not delete individually goes with it), so it must not be
        // reachable through an ordinary write capability. `create_collection`
        // stays ungated deliberately: a never-yet-existing collection has no
        // policy-protected rows to destroy, and an owner-rooted UCAN chain
        // can never carry `data-layer/admin` at all (`router/src/route_
        // handler/io.rs`'s `is_root` excludes admin-entailing capabilities
        // from per-service owner-rooting on purpose) -- gating creation too
        // would make it impossible for a service on an unowned substrate to
        // provision its own schema at all.
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
        store.drop_collection(&name).await
    }

    async fn put(
        &mut self,
        collection: String,
        value: RecordWriteValue,
    ) -> Result<(), DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        // Owned locals first -- `resolve_query_auth` takes `&mut self` and
        // the returned `QueryAuth<'_>` borrows it, so nothing may touch
        // `self` afterwards. Same discipline `get`/`query` already document
        // above.
        let creator_id = self.caller.write_attribution(&self.component_id);
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_WRITE.to_string()),
                Mode::Filter,
            )
            .await?;
        store.put(&collection, &value, &creator_id, query_auth.as_ref()).await
    }

    async fn patch(
        &mut self,
        collection: String,
        id: String,
        patch_json: Vec<u8>,
    ) -> Result<(), DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_WRITE.to_string()),
                Mode::Filter,
            )
            .await?;
        store.patch(&collection, &id, &patch_json, query_auth.as_ref()).await
    }

    async fn get(
        &mut self,
        collection: String,
        id: String,
    ) -> Result<Option<RecordReadValue>, DataLayerError> {
        // Copied into owned locals up front, before `resolve_query_auth`'s
        // `&mut self` borrow starts (its returned `QueryAuth<'_>` ties to
        // that borrow, so `self` cannot be touched again while it's held) --
        // same `Send`-future discipline `resolve_query_auth`'s own doc
        // comment describes, applied one step earlier.
        let session = self.caller.session.clone();
        let service_id = self.component_id.clone();
        let authorizer = self.row_authorizer.upgrade();

        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_READ.to_string()),
                Mode::PointInTime { id: id.clone() },
            )
            .await?;
        let outcome = store.get(&collection, &id, query_auth.as_ref()).await?;
        let Some(record) = outcome.value else { return Ok(None) };

        let Some(sieve) = query_auth.as_ref().and_then(|a| a.resolved_sieve.as_ref()) else {
            return strip_record(record, &outcome.masked_fields).map(Some);
        };
        if sieve.abac_permissions.is_empty() {
            return strip_record(record, &outcome.masked_fields).map(Some);
        }
        let candidate = to_candidate_row(&record);
        // Fail-closed, but distinguishably (B4-04): an after-step error
        // (pool exhaustion, a trap, a budget overrun) is not the same claim
        // as "the after-step ran and denied this row" -- only the latter is
        // `Ok(None)`.
        let kept =
            apply_stage4(sieve, &session, &service_id, &collection, authorizer, vec![candidate])
                .await
                .map_err(map_abac_error)?;
        let Some((_, extra)) = kept.into_iter().next() else { return Ok(None) };
        let masked = union_masked_fields(&outcome.masked_fields, extra);
        strip_record(record, &masked).map(Some)
    }

    async fn query(
        &mut self,
        collection: String,
        opts: QueryOptions,
    ) -> Result<QueryResult, DataLayerError> {
        // See `get`'s identical comment on why these are captured before
        // `resolve_query_auth` runs.
        let session = self.caller.session.clone();
        let service_id = self.component_id.clone();
        let authorizer = self.row_authorizer.upgrade();

        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_READ.to_string()),
                Mode::Filter,
            )
            .await?;
        let mut outcome = store.query(&collection, &opts, query_auth.as_ref()).await?;
        let rows = mem::take(&mut outcome.value.records);

        let sieve = query_auth.as_ref().and_then(|a| a.resolved_sieve.as_ref());
        let kept: Vec<(RecordReadValue, Vec<String>)> = match sieve {
            Some(sieve) if !sieve.abac_permissions.is_empty() => {
                let candidates: Vec<CandidateRow> = rows.iter().map(to_candidate_row).collect();
                match apply_stage4(
                    sieve,
                    &session,
                    &service_id,
                    &collection,
                    authorizer,
                    candidates,
                )
                .await
                {
                    // `kept` already excludes denied rows -- rebuild each
                    // surviving `RecordReadValue` from its candidate rather
                    // than trying to re-align against the original `rows`.
                    Ok(kept) => kept
                        .into_iter()
                        .map(|(row, extra)| (from_candidate_row(row), extra))
                        .collect(),
                    // Fail-closed, but as a distinguishable error, not a
                    // silent empty-and-successful page (B4-04): clearing
                    // `next_cursor` here would make `records: []` +
                    // `next_cursor: None` read as "no more pages", which is
                    // exactly the wrong signal for an after-step that
                    // couldn't run, as opposed to one that ran and denied
                    // every row.
                    Err(e) => {
                        return Err(map_abac_error(e));
                    }
                }
            }
            _ => rows.into_iter().map(|r| (r, Vec::new())).collect(),
        };

        let records = kept
            .into_iter()
            .map(|(record, extra)| {
                let masked = union_masked_fields(&outcome.masked_fields, extra);
                strip_record(record, &masked)
            })
            .collect::<Result<Vec<_>, _>>()?;
        outcome.value.records = records;
        Ok(outcome.value)
    }

    async fn aggregate(
        &mut self,
        collection: String,
        pipeline: String,
    ) -> Result<RawQueryResult, DataLayerError> {
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_READ.to_string()),
                Mode::Filter,
            )
            .await?;
        store.aggregate(&collection, &pipeline, query_auth.as_ref()).await
    }

    async fn delete(&mut self, collection: String, id: String) -> Result<(), DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_WRITE.to_string()),
                Mode::Filter,
            )
            .await?;
        store.delete(&collection, &id, query_auth.as_ref()).await
    }

    async fn delete_many(
        &mut self,
        collection: String,
        filter: String,
    ) -> Result<u64, DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_WRITE.to_string()),
                Mode::Filter,
            )
            .await?;
        store.delete_many(&collection, Some(filter.as_str()), query_auth.as_ref()).await
    }

    /// Mode A point-in-time authorization check (ADR-0017 §4). No capability
    /// gate, unlike `execute_ddl`/`query_raw`: `check-access` *is* the
    /// authorization primitive, reveals only the caller's own access, and is
    /// fail-closed to `false` inside the store -- gating it would be
    /// circular.
    async fn check_access(
        &mut self,
        collection: String,
        id: String,
        operation: String,
    ) -> Result<bool, DataLayerError> {
        // See `get`'s comment on why these are captured before
        // `resolve_query_auth`'s `&mut self` borrow starts.
        let session = self.caller.session.clone();
        let service_id = self.component_id.clone();
        let authorizer = self.row_authorizer.upgrade();

        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        // Fail-closed to `Ok(false)` on any resolution error (including a
        // cross-service fetch failure), matching Mode A's existing
        // `PolicyError`-compile-failure convention -- unlike Mode B's
        // `get`/`query`/`aggregate`/`delete_many`, a broken/undecidable
        // policy read is "no access", not a hard error.
        let query_auth = match self
            .resolve_query_auth(
                &collection,
                &Ability(operation.clone()),
                Mode::PointInTime { id: id.clone() },
            )
            .await
        {
            Ok(auth) => auth,
            Err(_) => return Ok(false),
        };
        let sieve = query_auth.as_ref().and_then(|a| a.resolved_sieve.as_ref());
        match sieve {
            Some(sieve) if !sieve.abac_permissions.is_empty() => {
                // A `get` under this `Mode::PointInTime` sieve runs exactly
                // the predicate `check_access` would have run, and
                // additionally hands back the row -- required to ask the
                // after-step (ADR-0017 §7) the same question `check-access`
                // does: "may this caller reach the row".
                let outcome = match store.get(&collection, &id, query_auth.as_ref()).await {
                    Ok(o) => o,
                    Err(_) => return Ok(false),
                };
                let Some(record) = outcome.value else { return Ok(false) };
                let candidate = to_candidate_row(&record);
                let kept = apply_stage4(
                    sieve,
                    &session,
                    &service_id,
                    &collection,
                    authorizer,
                    vec![candidate],
                )
                .await;
                // A `redact` decision counts as reachable: the question is
                // "may this caller reach the row", and a redacted row was
                // reached. `Err` or an empty `kept` (a `deny` decision) is
                // `false`.
                Ok(matches!(kept, Ok(k) if !k.is_empty()))
            }
            _ => store.check_access(&collection, &id, &operation, query_auth.as_ref()).await,
        }
    }

    async fn batch_mutate(
        &mut self,
        collection: String,
        mutations: Vec<Mutation>,
    ) -> Result<(), DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        // Owned locals first -- see `put`'s identical comment.
        let creator_id = self.caller.write_attribution(&self.component_id);
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_WRITE.to_string()),
                Mode::Filter,
            )
            .await?;
        store.batch_mutate(&collection, &mutations, &creator_id, query_auth.as_ref()).await
    }

    async fn execute_ddl(&mut self, sql: String) -> Result<(), DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
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
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
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

/// Ceiling on a guest-supplied idempotency key. It travels on every
/// attempt and becomes part of a primary key on the receiving node, so it
/// is bounded like any other guest-controlled string that leaves the
/// sandbox. Generous next to any real key (a UUID is 36 bytes).
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;

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
        RpcProxyError::Callee { code, message, data } => proxy::ProxyError::Callee(CalleeError {
            code,
            message,
            data: data.map(|v| v.to_string()),
        }),
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
        target: CallTarget,
        interface: String,
        method: String,
        params: String,
        options: Option<CallOptions>,
    ) -> Result<String, proxy::ProxyError> {
        // ADR-0017 §7 is *local* read-only lookups; a cross-service call
        // mid-query is exactly the N+1-over-the-network cost the ADR
        // deliberately contained by keeping stage 4 out of the query-planner
        // business.
        if self.read_only {
            return Err(proxy::ProxyError::Internal(
                "stage-4 after-step instances may not originate proxy calls".to_string(),
            ));
        }
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

        let (protocol_tag, idempotent, timeout_ms, routing_key, idempotency_key) = match &options {
            Some(o) => (
                o.protocol.as_deref(),
                o.idempotent,
                o.timeout_ms,
                o.routing_key.clone(),
                o.idempotency_key.clone(),
            ),
            None => (None, false, None, None, None),
        };
        let protocol =
            ProxyProtocol::parse(protocol_tag).map_err(proxy::ProxyError::UnsupportedProtocol)?;

        // ADR-0021 §2: the host supplies `app_instance_id`, the guest
        // supplies only the declared name. Resolution happens here, before
        // the `ProxyRequest` exists, so a guest never holds the resolved DID
        // and cannot snapshot it past a re-push.
        let target_service = match target {
            CallTarget::Service(service) => service,
            CallTarget::Dependency(name) => {
                let app_instance_id = self.app_instance_id.as_deref().ok_or_else(|| {
                    proxy::ProxyError::DependencyNotBound(format!(
                        "component '{}' was not deployed as part of an app instance, so it has no \
                         declared dependency '{name}'",
                        self.component_id
                    ))
                })?;
                let topology_key = TopologyKey::local(
                    // This string came out of a `service_app_context` row,
                    // not out of the guest. A corrupted row is a
                    // substrate-side fault, so it maps to `Internal`, not to
                    // the guest-facing "you are not bound".
                    AppInstanceId::try_new(app_instance_id).map_err(|e| {
                        proxy::ProxyError::Internal(format!(
                            "stored app context for '{}' is unreadable: {e}",
                            self.component_id
                        ))
                    })?,
                    LogicalServiceName::try_new(&name).map_err(|e| {
                        proxy::ProxyError::DependencyNotBound(format!(
                            "invalid dependency name: {e}"
                        ))
                    })?,
                );
                self.logical_resolver
                    .resolve(&topology_key, routing_key.as_deref().map(str::as_bytes))
                    .map_err(|e| {
                        proxy::ProxyError::DependencyNotBound(format!(
                            "dependency '{name}' of '{}' is not bound: {e}",
                            self.component_id
                        ))
                    })?
                    .to_string()
            }
        };

        // D-04-02-h ingress (ii): a guest proxying into its **own** service's
        // native `data-layer` forwards this invocation's real `HostState.
        // caller` (router-verified, or `service_system` if none reached
        // this guest -- see `prepare_wasm_execution`), so the receiving
        // `SynSvcNativeService::resolve_query_auth` sees who is actually
        // asking instead of always synthesizing `service_system` -- the
        // same-service exception (`ProxyRouter::check_native_capability_
        // gate`) already restricts this to the guest's own data, so this
        // cannot escalate to another service's rights.
        //
        // A genuine cross-service call still acts as itself: it does NOT
        // inherit the identity of whoever invoked *this* guest (no U->X
        // delegation exists in B0's model), so a proxied call to a
        // *different* service cannot be used to escalate to the original
        // caller's rights. Real cross-service caller-delegation is B1/UCAN,
        // not yet built. The self-proxy caller-forwarding rule is evaluated
        // against the *resolved* target: a component that reaches its own
        // service through a declared dependency name is still the same
        // service, so it still forwards its real caller.
        let caller = if target_service == self.component_id {
            self.caller.clone()
        } else {
            CallerContext::service_system(&self.component_id)
        };
        let req = ProxyRequest {
            target_service,
            interface,
            method,
            params,
            caller,
            origin: CallOrigin::Guest { service_id: self.component_id.clone() },
            protocol,
            idempotent,
            idempotency_key,
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

    /// Hands a call to this service's durable outbox.
    ///
    /// Unlike `call`, a dependency name is **not** resolved here: the queued
    /// item stores the name, and resolution happens again on every delivery
    /// attempt, so a binding re-pushed while the item waits takes effect
    /// (ADR-0021 §2). Resolving once and storing the answer would snapshot
    /// the resolved DID for hours -- the exact thing that rule forbids, and
    /// for far longer than a guest could manage on its own.
    async fn enqueue(
        &mut self,
        target: CallTarget,
        interface: String,
        method: String,
        params: String,
        options: Option<CallOptions>,
    ) -> Result<(), proxy::ProxyError> {
        if self.read_only {
            return Err(proxy::ProxyError::Internal(
                "stage-4 after-step instances may not originate proxy calls".to_string(),
            ));
        }
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

        let options = options.unwrap_or(CallOptions {
            protocol: None,
            idempotent: false,
            timeout_ms: None,
            routing_key: None,
            idempotency_key: None,
        });

        // Refused before anything is resolved, written, or attempted. A
        // queued call is delivered at least once, so one with no fence
        // would run the target twice on the first retry -- there is no
        // safe way to accept this, and naming the missing field is what
        // makes the refusal actionable.
        let idempotency_key = options.idempotency_key.clone().ok_or_else(|| {
            proxy::ProxyError::Internal(
                "enqueue requires call-options.idempotency-key: a queued call is delivered at \
                 least once, so it must carry a key the receiver can deduplicate on"
                    .to_string(),
            )
        })?;
        // Present is not the same as usable. An empty key is the sharp
        // case: it becomes this service's queue key, so the *first*
        // enqueue takes it and every later one is silently treated as a
        // duplicate of that one and dropped -- a guest would see nothing
        // but success while nothing was ever queued. The length bound is
        // the ordinary one for a guest-controlled string that becomes a
        // primary-key component on the receiving node.
        if idempotency_key.trim().is_empty() {
            return Err(proxy::ProxyError::Internal(
                "enqueue requires a non-empty call-options.idempotency-key: an empty key would be \
                 shared by every call this service queues"
                    .to_string(),
            ));
        }
        if idempotency_key.len() > MAX_IDEMPOTENCY_KEY_BYTES {
            return Err(proxy::ProxyError::Internal(format!(
                "call-options.idempotency-key is {} bytes; the limit is \
                 {MAX_IDEMPOTENCY_KEY_BYTES}",
                idempotency_key.len()
            )));
        }

        // Validated here so an unsupported tag fails at the call rather
        // than hours later in a dead letter.
        ProxyProtocol::parse(options.protocol.as_deref())
            .map_err(proxy::ProxyError::UnsupportedProtocol)?;

        let target = match target {
            CallTarget::Service(service) => QueuedTarget::Service(service),
            CallTarget::Dependency(name) => {
                // Validated now, resolved later: an unusable name should
                // fail at the call, but the resolution itself must happen
                // at delivery.
                LogicalServiceName::try_new(&name).map_err(|e| {
                    proxy::ProxyError::DependencyNotBound(format!("invalid dependency name: {e}"))
                })?;
                if self.app_instance_id.is_none() {
                    return Err(proxy::ProxyError::DependencyNotBound(format!(
                        "component '{}' was not deployed as part of an app instance, so it has no \
                         declared dependency '{name}'",
                        self.component_id
                    )));
                }
                QueuedTarget::Dependency(name)
            }
        };

        service_proxy
            .enqueue(QueuedCall {
                app_instance_id: self.app_instance_id.clone(),
                caller_service_id: self.component_id.clone(),
                target,
                routing_key: options.routing_key,
                interface,
                method,
                params,
                idempotency_key,
                protocol: options.protocol,
                timeout_ms: options.timeout_ms.map(u64::from),
            })
            .await
            .map_err(map_proxy_error)
    }
}

fn rpc_saga_state_to_wit(state: RpcSagaState) -> WitSagaState {
    match state {
        RpcSagaState::Open => WitSagaState::Open,
        RpcSagaState::Compensating => WitSagaState::Compensating,
        RpcSagaState::Compensated => WitSagaState::Compensated,
        RpcSagaState::Failed => WitSagaState::Failed,
    }
}

impl saga::Host for HostState {
    /// Opens a saga (ADR-0023 §7, as amended). Each of this interface's
    /// five functions follows `proxy::Host::call`'s own shape:
    /// the `read_only` refusal first, then `service_proxy.upgrade()`, then
    /// params parsing, then the call.
    async fn begin(
        &mut self,
        name: String,
        deadline_secs: Option<u64>,
    ) -> Result<String, proxy::ProxyError> {
        if self.read_only {
            return Err(proxy::ProxyError::Internal(
                "stage-4 after-step instances may not originate proxy calls".to_string(),
            ));
        }
        let service_proxy = self
            .service_proxy
            .upgrade()
            .ok_or_else(|| proxy::ProxyError::Internal("proxy unavailable".to_string()))?;
        service_proxy
            .saga_begin(SagaBegin {
                caller_service_id: self.component_id.clone(),
                app_instance_id: self.app_instance_id.clone(),
                name,
                deadline_secs,
            })
            .await
            .map_err(map_proxy_error)
    }

    /// Takes one forward step. Follows `enqueue`'s own target handling
    /// exactly: a dependency name is validated, never resolved here --
    /// resolution happens host-side, at the moment of dispatch.
    async fn step(
        &mut self,
        saga_id: String,
        target: CallTarget,
        interface: String,
        method: String,
        params: String,
        options: Option<CallOptions>,
    ) -> Result<String, proxy::ProxyError> {
        if self.read_only {
            return Err(proxy::ProxyError::Internal(
                "stage-4 after-step instances may not originate proxy calls".to_string(),
            ));
        }
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

        let options = options.unwrap_or(CallOptions {
            protocol: None,
            idempotent: false,
            timeout_ms: None,
            routing_key: None,
            idempotency_key: None,
        });
        ProxyProtocol::parse(options.protocol.as_deref())
            .map_err(proxy::ProxyError::UnsupportedProtocol)?;

        let target = match target {
            CallTarget::Service(service) => QueuedTarget::Service(service),
            CallTarget::Dependency(name) => {
                LogicalServiceName::try_new(&name).map_err(|e| {
                    proxy::ProxyError::DependencyNotBound(format!("invalid dependency name: {e}"))
                })?;
                if self.app_instance_id.is_none() {
                    return Err(proxy::ProxyError::DependencyNotBound(format!(
                        "component '{}' was not deployed as part of an app instance, so it has no \
                         declared dependency '{name}'",
                        self.component_id
                    )));
                }
                QueuedTarget::Dependency(name)
            }
        };

        let value = service_proxy
            .saga_step(SagaStepRequest {
                caller_service_id: self.component_id.clone(),
                app_instance_id: self.app_instance_id.clone(),
                saga_id,
                target,
                routing_key: options.routing_key,
                interface,
                method,
                params,
                idempotency_key: options.idempotency_key,
                protocol: options.protocol,
                timeout_ms: options.timeout_ms.map(u64::from),
            })
            .await
            .map_err(map_proxy_error)?;
        Ok(match value {
            Value::String(s) => s,
            other => other.to_string(),
        })
    }

    async fn commit(&mut self, saga_id: String) -> Result<(), proxy::ProxyError> {
        if self.read_only {
            return Err(proxy::ProxyError::Internal(
                "stage-4 after-step instances may not originate proxy calls".to_string(),
            ));
        }
        let service_proxy = self
            .service_proxy
            .upgrade()
            .ok_or_else(|| proxy::ProxyError::Internal("proxy unavailable".to_string()))?;
        service_proxy.saga_commit(&self.component_id, &saga_id).await.map_err(map_proxy_error)
    }

    async fn compensate(&mut self, saga_id: String) -> Result<(), proxy::ProxyError> {
        if self.read_only {
            return Err(proxy::ProxyError::Internal(
                "stage-4 after-step instances may not originate proxy calls".to_string(),
            ));
        }
        let service_proxy = self
            .service_proxy
            .upgrade()
            .ok_or_else(|| proxy::ProxyError::Internal("proxy unavailable".to_string()))?;
        service_proxy.saga_compensate(&self.component_id, &saga_id).await.map_err(map_proxy_error)
    }

    async fn status(&mut self, saga_id: String) -> Result<SagaStatus, proxy::ProxyError> {
        if self.read_only {
            return Err(proxy::ProxyError::Internal(
                "stage-4 after-step instances may not originate proxy calls".to_string(),
            ));
        }
        let service_proxy = self
            .service_proxy
            .upgrade()
            .ok_or_else(|| proxy::ProxyError::Internal("proxy unavailable".to_string()))?;
        let info = service_proxy
            .saga_status(&self.component_id, &saga_id)
            .await
            .map_err(map_proxy_error)?;
        Ok(SagaStatus {
            saga_id: info.saga_id,
            name: info.name,
            state: rpc_saga_state_to_wit(info.state),
            steps: info.steps,
            compensated_steps: info.compensated_steps,
            created_at: info.created_at,
            deadline_at: info.deadline_at,
            last_error: info.last_error,
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
        if self.read_only {
            return Err(BlobError::Internal(
                "stage-4 after-step instances are read-only".to_string(),
            ));
        }
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
        if self.read_only {
            return Err(BlobError::Internal(
                "stage-4 after-step instances are read-only".to_string(),
            ));
        }
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
        if self.read_only {
            return Err(BlobError::Internal(
                "stage-4 after-step instances are read-only".to_string(),
            ));
        }
        self.blob_provider.delete_blob(&self.component_id, &hash).await.map_err(map_blob_error)
    }

    async fn signed_url(&mut self, hash: String, ttl_secs: u32) -> Result<String, BlobError> {
        // Every other mutating/egress host function is hard-denied under
        // `read_only` (review finding B4-14); a signed URL is a read in
        // shape but mints a time-limited, externally redeemable URL that
        // outlives this throw-away stage-4 instance -- the same kind of
        // egress-beyond-the-call ADR-0017 §7's "local, read-only lookups
        // only" is meant to rule out.
        if self.read_only {
            return Err(BlobError::Internal(
                "stage-4 after-step instances are read-only".to_string(),
            ));
        }
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
        if self.read_only {
            return Err(BlobError::Internal(
                "stage-4 after-step instances are read-only".to_string(),
            ));
        }
        let session = self.table.get_mut(&self_).map_err(|e| BlobError::Internal(e.to_string()))?;
        session.0.write(chunk).await.map_err(map_blob_error)
    }

    async fn finish(&mut self, self_: Resource<BlobWriter>) -> Result<String, BlobError> {
        if self.read_only {
            return Err(BlobError::Internal(
                "stage-4 after-step instances are read-only".to_string(),
            ));
        }
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

impl syneroym_wit_interfaces::http::syneroym::http::websocket::Host for HostState {
    async fn send(
        &mut self,
        conn: String,
        frame: Vec<u8>,
        kind: syneroym_wit_interfaces::http::syneroym::http::websocket_types::FrameKind,
    ) -> Result<(), String> {
        if let Some(engine) = self.messaging.engine.upgrade()
            && let Some(service_map) = engine.websocket_senders.get(&self.component_id)
            && let Some(sender) = service_map.get(&conn)
        {
            match sender.send((frame, kind)).await {
                Ok(_) => return Ok(()),
                Err(_) => return Err("Connection closed".to_string()),
            }
        }
        Err("Unknown connection ID".to_string())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use serde_json::json;
    use syneroym_core::{local_registry::EndpointRegistry, storage::MockStorage};
    use syneroym_data_blob::ObjectStoreBlobProvider;
    use syneroym_data_db::SqliteStorageProvider;
    use syneroym_fdae::parse_and_validate;
    use syneroym_identity::substrate;
    use syneroym_mqtt_broker::MqttBrokerConfig;
    use syneroym_rpc::{Capability, SessionContext};

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

    /// Records the last `ProxyRequest` it was invoked with, so a test can
    /// inspect what `proxy::Host::call` actually built (in particular
    /// `caller`) without needing a real downstream service to answer.
    /// `invoke_count` lets a test pin the "no network hop" budget (plan §7):
    /// a dependency resolution that went through the router/supervisor
    /// instead of resolving host-side, before the `ProxyRequest` exists,
    /// would still land here, but with more than the one `invoke` a single
    /// call is supposed to cost.
    #[derive(Debug, Default)]
    struct RecordingProxy {
        last_request: Mutex<Option<ProxyRequest>>,
        invoke_count: AtomicUsize,
        last_enqueued: Mutex<Option<QueuedCall>>,
        enqueue_count: AtomicUsize,
        last_saga_begin: Mutex<Option<SagaBegin>>,
        last_saga_step: Mutex<Option<SagaStepRequest>>,
        last_saga_commit: Mutex<Option<(String, String)>>,
        last_saga_compensate: Mutex<Option<(String, String)>>,
    }

    #[async_trait::async_trait]
    impl ServiceProxy for RecordingProxy {
        async fn invoke(&self, request: ProxyRequest) -> Result<Value, RpcProxyError> {
            self.invoke_count.fetch_add(1, Ordering::SeqCst);
            let recorded = request.clone();
            *self.last_request.lock().unwrap() = Some(recorded);
            Ok(Value::Null)
        }

        async fn enqueue(&self, call: QueuedCall) -> Result<(), RpcProxyError> {
            self.enqueue_count.fetch_add(1, Ordering::SeqCst);
            *self.last_enqueued.lock().unwrap() = Some(call);
            Ok(())
        }

        async fn saga_begin(&self, req: SagaBegin) -> Result<String, RpcProxyError> {
            *self.last_saga_begin.lock().unwrap() = Some(req);
            Ok("saga-1".to_string())
        }

        async fn saga_step(&self, req: SagaStepRequest) -> Result<Value, RpcProxyError> {
            *self.last_saga_step.lock().unwrap() = Some(req);
            Ok(Value::Null)
        }

        async fn saga_commit(&self, service_id: &str, saga_id: &str) -> Result<(), RpcProxyError> {
            *self.last_saga_commit.lock().unwrap() =
                Some((service_id.to_string(), saga_id.to_string()));
            Ok(())
        }

        async fn saga_compensate(
            &self,
            service_id: &str,
            saga_id: &str,
        ) -> Result<(), RpcProxyError> {
            *self.last_saga_compensate.lock().unwrap() =
                Some((service_id.to_string(), saga_id.to_string()));
            Ok(())
        }

        async fn saga_status(
            &self,
            _service_id: &str,
            saga_id: &str,
        ) -> Result<syneroym_rpc::SagaInfo, RpcProxyError> {
            Ok(syneroym_rpc::SagaInfo {
                saga_id: saga_id.to_string(),
                name: "wf".to_string(),
                state: RpcSagaState::Open,
                steps: 1,
                compensated_steps: 0,
                created_at: 1_000,
                deadline_at: 4_600_000,
                last_error: None,
            })
        }
    }

    fn enqueue_options(idempotency_key: Option<&str>) -> Option<CallOptions> {
        Some(CallOptions {
            protocol: None,
            idempotent: false,
            timeout_ms: None,
            routing_key: None,
            idempotency_key: idempotency_key.map(str::to_string),
        })
    }

    /// A queued call is delivered at least once, so one with no fence
    /// would run the target twice on the first retry. Refused before
    /// anything is resolved, written, or attempted -- and the refusal
    /// names the missing field, so a future slice relaxing this has to
    /// confront the argument.
    #[tokio::test]
    async fn an_enqueue_without_an_idempotency_key_is_refused() {
        let resolver = Arc::new(LogicalResolver::new(Arc::new(
            syneroym_app_orchestration::StaticInventory::new(),
        )));
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = dependency_host("frontend", None, resolver, &proxy, temp_dir.path());

        let err = proxy::Host::enqueue(
            &mut host,
            CallTarget::Service("did:key:zBackend".to_string()),
            "greeter".to_string(),
            "greet".to_string(),
            "null".to_string(),
            enqueue_options(None),
        )
        .await
        .expect_err("an unkeyed enqueue must be refused");

        let proxy::ProxyError::Internal(message) = err else {
            panic!("expected an internal refusal, got {err:?}");
        };
        assert!(
            message.contains("idempotency-key"),
            "the refusal must name the missing field, got: {message}"
        );
        assert_eq!(
            proxy.enqueue_count.load(Ordering::SeqCst),
            0,
            "nothing may reach the outbox for an unfenced call"
        );
    }

    /// Present is not usable. An empty key becomes this service's queue
    /// key, so the first enqueue would take it and every later one would
    /// be silently dropped as a duplicate -- the guest seeing success
    /// while nothing was queued.
    #[tokio::test]
    async fn an_enqueue_with_a_blank_idempotency_key_is_refused() {
        let resolver = Arc::new(LogicalResolver::new(Arc::new(
            syneroym_app_orchestration::StaticInventory::new(),
        )));
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = dependency_host("frontend", None, resolver, &proxy, temp_dir.path());

        for blank in ["", "   "] {
            let err = proxy::Host::enqueue(
                &mut host,
                CallTarget::Service("did:key:zBackend".to_string()),
                "greeter".to_string(),
                "greet".to_string(),
                "null".to_string(),
                enqueue_options(Some(blank)),
            )
            .await
            .expect_err("a blank key must be refused");
            assert!(
                matches!(err, proxy::ProxyError::Internal(ref m) if m.contains("non-empty")),
                "unexpected error for {blank:?}: {err:?}"
            );
        }
        assert_eq!(proxy.enqueue_count.load(Ordering::SeqCst), 0);
    }

    /// The key travels on every attempt and becomes part of a primary key
    /// on the receiving node, so it is bounded like any other
    /// guest-controlled string that leaves the sandbox.
    #[tokio::test]
    async fn an_over_long_idempotency_key_is_refused() {
        let resolver = Arc::new(LogicalResolver::new(Arc::new(
            syneroym_app_orchestration::StaticInventory::new(),
        )));
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = dependency_host("frontend", None, resolver, &proxy, temp_dir.path());

        let err = proxy::Host::enqueue(
            &mut host,
            CallTarget::Service("did:key:zBackend".to_string()),
            "greeter".to_string(),
            "greet".to_string(),
            "null".to_string(),
            enqueue_options(Some(&"k".repeat(MAX_IDEMPOTENCY_KEY_BYTES + 1))),
        )
        .await
        .expect_err("an over-long key must be refused");
        assert!(matches!(err, proxy::ProxyError::Internal(_)), "{err:?}");
        assert_eq!(proxy.enqueue_count.load(Ordering::SeqCst), 0);
    }

    /// The stored item names the dependency, never a resolved DID: the
    /// worker resolves again at every attempt, so a re-pushed binding
    /// takes effect (ADR-0021 §2).
    #[tokio::test]
    async fn an_enqueued_dependency_is_stored_by_name_not_resolved_at_the_host() {
        use syneroym_app_orchestration::AppRegistry;

        let registry = Arc::new(syneroym_app_orchestration::StaticInventory::new());
        registry.register(
            TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("backend")),
            dependency_topology_entry(vec!["did:key:zBackendMember"]),
        );
        let resolver = Arc::new(LogicalResolver::new(registry));
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = dependency_host(
            "frontend",
            Some("app-1".to_string()),
            resolver,
            &proxy,
            temp_dir.path(),
        );

        proxy::Host::enqueue(
            &mut host,
            CallTarget::Dependency("backend".to_string()),
            "greeter".to_string(),
            "greet".to_string(),
            "null".to_string(),
            enqueue_options(Some("msg-7")),
        )
        .await
        .unwrap();

        let stored = proxy.last_enqueued.lock().unwrap().take().unwrap();
        assert_eq!(stored.target, QueuedTarget::Dependency("backend".to_string()));
        assert_eq!(stored.idempotency_key, "msg-7");
        assert_eq!(stored.app_instance_id.as_deref(), Some("app-1"));
        assert_eq!(stored.caller_service_id, "frontend");
    }

    /// D-04-02-h ingress (ii)'s self-proxy forwarding (`proxy::Host::call`)
    /// is scoped to `service == self.component_id` -- a genuinely
    /// cross-service proxy call must still synthesize `service_system`, per
    /// the function's own doc comment ("does NOT inherit the identity of
    /// whoever invoked *this* guest"). Nothing pinned that fact before this
    /// test; the whole "cannot escalate to another service's rights"
    /// argument in the doc comment rested on it being true, unverified.
    #[tokio::test]
    async fn self_proxy_forwarding_does_not_extend_to_a_different_target_service() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let proxy = Arc::new(RecordingProxy::default());

        let real_caller = CallerContext {
            caller_did: "did:key:zRealCaller".to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: "did:key:zRealCaller".to_string(),
                capabilities: vec![Capability {
                    with: ResourceUri::substrate("did:key:zRealCaller"),
                    can: Ability(Ability::SUBSTRATE_ADMIN.to_string()),
                    caveats: None,
                }],
                ..Default::default()
            },
            auth: AuthLevel::Ucan,
            proof: None,
        };

        let mut host = HostState::new(
            "svc-a".to_string(),
            None,
            Arc::new(KeyStore::new()),
            storage,
            test_blob_provider(),
            real_caller,
            0,
            test_messaging_context(),
            test_streaming_context(),
            Arc::downgrade(&proxy) as Weak<dyn ServiceProxy>,
            None,
            false,
            syneroym_rpc::empty_row_authorizer(),
            None,
            syneroym_app_orchestration::empty_resolver(),
        );

        proxy::Host::call(
            &mut host,
            CallTarget::Service("svc-b".to_string()),
            "some-interface".to_string(),
            "some-method".to_string(),
            "null".to_string(),
            None,
        )
        .await
        .unwrap();

        let received = proxy.last_request.lock().unwrap().take().unwrap();
        assert_eq!(
            received.caller.auth,
            AuthLevel::System,
            "a proxy call to a *different* service must not carry the guest's real caller \
             identity, capabilities included -- got {:?}",
            received.caller
        );
        assert!(
            received.caller.session.capabilities.is_empty(),
            "a cross-service proxy call must never carry the guest's real capabilities: {:?}",
            received.caller.session.capabilities
        );
    }

    // ── A2: dependency resolution through `proxy::Host::call` ──────────

    fn dependency_topology_entry(members: Vec<&str>) -> syneroym_app_orchestration::TopologyEntry {
        syneroym_app_orchestration::TopologyEntry {
            mode: if members.len() > 1 {
                syneroym_app_orchestration::TopologyMode::Redundant
            } else {
                syneroym_app_orchestration::TopologyMode::Singleton
            },
            members: members.into_iter().map(syneroym_app_orchestration::ServiceId::new).collect(),
            sharding_strategy: None,
            epoch: syneroym_app_orchestration::TopologyEpoch::default(),
            cache_ttl: Duration::from_secs(60),
            not_after: None,
        }
    }

    /// Builds a `HostState` naming `component_id` as deployed under
    /// `app_instance_id` (or standalone, if `None`), backed by `resolver`
    /// and `proxy`. `db_dir` must outlive the returned `HostState`.
    fn dependency_host(
        component_id: &str,
        app_instance_id: Option<String>,
        resolver: Arc<LogicalResolver>,
        proxy: &Arc<RecordingProxy>,
        db_dir: &std::path::Path,
    ) -> HostState {
        HostState::new(
            component_id.to_string(),
            None,
            Arc::new(KeyStore::new()),
            Arc::new(SqliteStorageProvider::new(db_dir, false).unwrap()),
            test_blob_provider(),
            CallerContext::service_system(component_id),
            0,
            test_messaging_context(),
            test_streaming_context(),
            Arc::downgrade(proxy) as Weak<dyn ServiceProxy>,
            None,
            false,
            syneroym_rpc::empty_row_authorizer(),
            app_instance_id,
            resolver,
        )
    }

    #[tokio::test]
    async fn a_dependency_name_resolves_to_its_bound_member_before_the_request_is_built() {
        use syneroym_app_orchestration::AppRegistry;

        let registry = Arc::new(syneroym_app_orchestration::StaticInventory::new());
        registry.register(
            TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("backend")),
            dependency_topology_entry(vec!["did:key:zBackendMember"]),
        );
        let resolver = Arc::new(LogicalResolver::new(registry));
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = dependency_host(
            "frontend",
            Some("app-1".to_string()),
            resolver,
            &proxy,
            temp_dir.path(),
        );

        proxy::Host::call(
            &mut host,
            CallTarget::Dependency("backend".to_string()),
            "greeter".to_string(),
            "greet".to_string(),
            "null".to_string(),
            None,
        )
        .await
        .unwrap();

        let received = proxy.last_request.lock().unwrap().take().unwrap();
        assert_eq!(received.target_service, "did:key:zBackendMember");
        // Plan §7's "no network hop" budget: dependency resolution happens
        // host-side, before the `ProxyRequest` is built, so one dependency
        // call must cost exactly one `invoke` -- never a second hop to ask
        // a supervisor or router to resolve it (ADR-0021 §8 forbids that
        // outright).
        assert_eq!(proxy.invoke_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_unbound_dependency_name_is_dependency_not_bound_and_never_reaches_the_proxy() {
        let resolver = syneroym_app_orchestration::empty_resolver();
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = dependency_host(
            "frontend",
            Some("app-1".to_string()),
            resolver,
            &proxy,
            temp_dir.path(),
        );

        let err = proxy::Host::call(
            &mut host,
            CallTarget::Dependency("backend".to_string()),
            "greeter".to_string(),
            "greet".to_string(),
            "null".to_string(),
            None,
        )
        .await
        .unwrap_err();

        assert!(
            matches!(err, proxy::ProxyError::DependencyNotBound(_)),
            "an unbound dependency name must fail as dependency-not-bound, not service-not-found: \
             {err:?}"
        );
        assert!(
            proxy.last_request.lock().unwrap().is_none(),
            "resolution must fail before a ProxyRequest is ever built"
        );
    }

    #[tokio::test]
    async fn a_component_with_no_app_context_cannot_name_a_dependency() {
        let resolver = syneroym_app_orchestration::empty_resolver();
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = dependency_host("standalone-svc", None, resolver, &proxy, temp_dir.path());

        let err = proxy::Host::call(
            &mut host,
            CallTarget::Dependency("backend".to_string()),
            "greeter".to_string(),
            "greet".to_string(),
            "null".to_string(),
            None,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, proxy::ProxyError::DependencyNotBound(_)));
    }

    #[tokio::test]
    async fn a_raw_did_target_is_unchanged() {
        let resolver = syneroym_app_orchestration::empty_resolver();
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = dependency_host(
            "frontend",
            Some("app-1".to_string()),
            resolver,
            &proxy,
            temp_dir.path(),
        );

        proxy::Host::call(
            &mut host,
            CallTarget::Service("did:key:zSomeoneElse".to_string()),
            "greeter".to_string(),
            "greet".to_string(),
            "null".to_string(),
            None,
        )
        .await
        .unwrap();

        let received = proxy.last_request.lock().unwrap().take().unwrap();
        assert_eq!(received.target_service, "did:key:zSomeoneElse");
    }

    #[tokio::test]
    async fn a_routing_key_selects_deterministically_across_a_two_member_binding() {
        use syneroym_app_orchestration::AppRegistry;

        let registry = Arc::new(syneroym_app_orchestration::StaticInventory::new());
        registry.register(
            TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("backend")),
            dependency_topology_entry(vec!["did:key:zMemberA", "did:key:zMemberB"]),
        );
        let resolver = Arc::new(LogicalResolver::new(registry));
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = dependency_host(
            "frontend",
            Some("app-1".to_string()),
            resolver,
            &proxy,
            temp_dir.path(),
        );

        let options = Some(CallOptions {
            protocol: None,
            idempotent: false,
            timeout_ms: None,
            routing_key: Some("user-42".to_string()),
            idempotency_key: None,
        });
        proxy::Host::call(
            &mut host,
            CallTarget::Dependency("backend".to_string()),
            "greeter".to_string(),
            "greet".to_string(),
            "null".to_string(),
            options.clone(),
        )
        .await
        .unwrap();
        let first = proxy.last_request.lock().unwrap().take().unwrap().target_service;

        proxy::Host::call(
            &mut host,
            CallTarget::Dependency("backend".to_string()),
            "greeter".to_string(),
            "greet".to_string(),
            "null".to_string(),
            options,
        )
        .await
        .unwrap();
        let second = proxy.last_request.lock().unwrap().take().unwrap().target_service;

        assert_eq!(first, second, "the same routing key must select the same member every time");
    }

    /// The guest's fence has to reach the request the router builds, or
    /// nothing downstream can put it on the wire for the receiver to
    /// dedup on (ADR-0023 §4).
    #[tokio::test]
    async fn the_host_function_passes_the_guests_key_into_the_proxy_request() {
        let registry = Arc::new(syneroym_app_orchestration::StaticInventory::new());
        let resolver = Arc::new(LogicalResolver::new(registry));
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = dependency_host("frontend", None, resolver, &proxy, temp_dir.path());

        proxy::Host::call(
            &mut host,
            CallTarget::Service("did:key:zBackend".to_string()),
            "greeter".to_string(),
            "greet".to_string(),
            "null".to_string(),
            Some(CallOptions {
                protocol: None,
                idempotent: false,
                timeout_ms: None,
                routing_key: None,
                idempotency_key: Some("msg-7".to_string()),
            }),
        )
        .await
        .unwrap();

        let received = proxy.last_request.lock().unwrap().take().unwrap();
        assert_eq!(received.idempotency_key.as_deref(), Some("msg-7"));
    }

    /// The ordinary call is unchanged: no options, no key.
    #[tokio::test]
    async fn a_call_with_no_options_carries_no_idempotency_key() {
        let registry = Arc::new(syneroym_app_orchestration::StaticInventory::new());
        let resolver = Arc::new(LogicalResolver::new(registry));
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = dependency_host("frontend", None, resolver, &proxy, temp_dir.path());

        proxy::Host::call(
            &mut host,
            CallTarget::Service("did:key:zBackend".to_string()),
            "greeter".to_string(),
            "greet".to_string(),
            "null".to_string(),
            None,
        )
        .await
        .unwrap();

        let received = proxy.last_request.lock().unwrap().take().unwrap();
        assert_eq!(received.idempotency_key, None);
    }

    #[tokio::test]
    async fn a_dependency_resolving_to_the_components_own_service_still_forwards_the_real_caller() {
        use syneroym_app_orchestration::AppRegistry;

        let registry = Arc::new(syneroym_app_orchestration::StaticInventory::new());
        registry.register(
            TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("self-dep")),
            dependency_topology_entry(vec!["did:key:zSelf"]),
        );
        let resolver = Arc::new(LogicalResolver::new(registry));
        let proxy = Arc::new(RecordingProxy::default());
        let real_caller = CallerContext {
            caller_did: "did:key:zRealCaller".to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: "did:key:zRealCaller".to_string(),
                ..Default::default()
            },
            auth: AuthLevel::Ucan,
            proof: None,
        };
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = HostState::new(
            "did:key:zSelf".to_string(),
            None,
            Arc::new(KeyStore::new()),
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap()),
            test_blob_provider(),
            real_caller,
            0,
            test_messaging_context(),
            test_streaming_context(),
            Arc::downgrade(&proxy) as Weak<dyn ServiceProxy>,
            None,
            false,
            syneroym_rpc::empty_row_authorizer(),
            Some("app-1".to_string()),
            resolver,
        );

        proxy::Host::call(
            &mut host,
            CallTarget::Dependency("self-dep".to_string()),
            "greeter".to_string(),
            "greet".to_string(),
            "null".to_string(),
            None,
        )
        .await
        .unwrap();

        let received = proxy.last_request.lock().unwrap().take().unwrap();
        assert_eq!(received.target_service, "did:key:zSelf");
        assert_eq!(
            received.caller.caller_did, "did:key:zRealCaller",
            "a dependency that resolves to the component's own service is still a self-proxy \
             call, and must forward the real caller"
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
            CallerContext::service_system("test-caller"),
            generation,
            test_messaging_context(),
            test_streaming_context(),
            test_service_proxy(),
            None,
            false,
            syneroym_rpc::empty_row_authorizer(),
            None,
            syneroym_app_orchestration::empty_resolver(),
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
            None,
            false,
            syneroym_rpc::empty_row_authorizer(),
            None,
            syneroym_app_orchestration::empty_resolver(),
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
            None,
            false,
            syneroym_rpc::empty_row_authorizer(),
            None,
            syneroym_app_orchestration::empty_resolver(),
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
            None,
            false,
            syneroym_rpc::empty_row_authorizer(),
            None,
            syneroym_app_orchestration::empty_resolver(),
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
        key_store.inject_kek([3u8; 32]).unwrap();
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
            None,
            false,
            syneroym_rpc::empty_row_authorizer(),
            None,
            syneroym_app_orchestration::empty_resolver(),
        );

        let result = vault::Host::reveal(&mut host_state, "does-not-exist".to_string()).await;
        assert!(matches!(result, Err(VaultError::NotFound)));
    }

    // -- FDAE host wiring (M04B Slice B2 Phase 3) --------------------------
    //
    // Real `QueryAuth` construction from `HostState.fdae_policy`/`caller`,
    // `check-access`, and host-side CLS field-stripping, exercised through
    // `store::Host` on a `HostState` built with a hand-injected `Policy`
    // (`fdae_policy` stays `None` in production until Phase 4).

    const FDAE_SERVICE_ID: &str = "svc-fdae-host-test";

    fn fdae_resource(collection: &str) -> ResourceUri {
        ResourceUri(format!(
            "{}/collection/{collection}",
            ResourceUri::service(FDAE_SERVICE_ID, FDAE_SERVICE_ID).0
        ))
    }

    fn fdae_read_cap(collection: &str) -> Capability {
        Capability {
            with: fdae_resource(collection),
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: None,
        }
    }

    fn fdae_caller(subject_did: &str, capabilities: Vec<Capability>) -> CallerContext {
        CallerContext {
            caller_did: subject_did.to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: subject_did.to_string(),
                capabilities,
                ..Default::default()
            },
            auth: AuthLevel::Ucan,
            proof: None,
        }
    }

    /// `document` --creator--> `user`, `view` permission reachable only via
    /// the creator relation. Mirrors `data_db::tests_fdae::single_hop_policy`.
    fn fdae_single_hop_policy() -> Policy {
        parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "view": {"allows": ["data-layer/read"], "paths": [["creator", "caller"]]}
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#,
        )
        .unwrap()
    }

    /// Same shape as `fdae_single_hop_policy`, plus a CLS `fields.deny:
    /// ["ssn"]`.
    fn fdae_cls_policy() -> Policy {
        parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "view": {
                                "allows": ["data-layer/read"],
                                "paths": [["creator", "caller"]],
                                "fields": {"deny": ["ssn"]}
                            }
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#,
        )
        .unwrap()
    }

    /// A `manage` permission covering `data-layer/write`, reachable via the
    /// same creator relation -- used to exercise `delete_many`'s write-mode
    /// sieve. Mirrors `data_db::tests_fdae::write_policy`.
    fn fdae_write_policy() -> Policy {
        parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "manage": {"allows": ["data-layer/write"], "paths": [["creator", "caller"]]}
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#,
        )
        .unwrap()
    }

    fn fdae_write_cap(collection: &str) -> Capability {
        Capability {
            with: fdae_resource(collection),
            can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
            caveats: None,
        }
    }

    fn fdae_host_state(
        storage_provider: Arc<dyn StorageProvider>,
        caller: CallerContext,
        fdae_policy: Option<Arc<Policy>>,
    ) -> HostState {
        HostState::new(
            FDAE_SERVICE_ID.to_string(),
            None,
            Arc::new(KeyStore::new()),
            storage_provider,
            test_blob_provider(),
            caller,
            0,
            test_messaging_context(),
            test_streaming_context(),
            test_service_proxy(),
            fdae_policy,
            false,
            syneroym_rpc::empty_row_authorizer(),
            None,
            syneroym_app_orchestration::empty_resolver(),
        )
    }

    /// Seeds `users`/`documents` collections: `doc-1` created by alice,
    /// `doc-2` created by bob, both carrying an `ssn` field for the CLS
    /// tests. Uses a policy-absent `HostState` (`put`/`create_collection`
    /// carry no FDAE gate).
    async fn fdae_seed_documents(storage_provider: Arc<dyn StorageProvider>) {
        let mut seeder =
            fdae_host_state(storage_provider, CallerContext::service_system(FDAE_SERVICE_ID), None);
        store::Host::create_collection(
            &mut seeder,
            CollectionSchema { name: "users".to_string(), indexes: vec![] },
        )
        .await
        .unwrap();
        store::Host::create_collection(
            &mut seeder,
            CollectionSchema { name: "documents".to_string(), indexes: vec![] },
        )
        .await
        .unwrap();
        store::Host::put(
            &mut seeder,
            "users".to_string(),
            RecordWriteValue {
                id: "u-alice".to_string(),
                payload: json!({"did": "did:key:alice"}).to_string().into_bytes(),
            },
        )
        .await
        .unwrap();
        store::Host::put(
            &mut seeder,
            "users".to_string(),
            RecordWriteValue {
                id: "u-bob".to_string(),
                payload: json!({"did": "did:key:bob"}).to_string().into_bytes(),
            },
        )
        .await
        .unwrap();
        store::Host::put(
            &mut seeder,
            "documents".to_string(),
            RecordWriteValue {
                id: "doc-1".to_string(),
                payload: json!({"creator_uuid": "u-alice", "ssn": "111-11-1111"})
                    .to_string()
                    .into_bytes(),
            },
        )
        .await
        .unwrap();
        store::Host::put(
            &mut seeder,
            "documents".to_string(),
            RecordWriteValue {
                id: "doc-2".to_string(),
                payload: json!({"creator_uuid": "u-bob", "ssn": "222-22-2222"})
                    .to_string()
                    .into_bytes(),
            },
        )
        .await
        .unwrap();
    }

    fn payload_json(record: &RecordReadValue) -> Value {
        serde_json::from_slice(&record.payload).unwrap()
    }

    /// RLS: `get`/`query` return only alice's own reachable row, and
    /// `check_access` matches (reachable -> `true`, unreachable -> `false`).
    #[tokio::test]
    async fn fdae_rls_filters_get_query_and_check_access() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        fdae_seed_documents(storage_provider.clone()).await;

        let policy = Arc::new(fdae_single_hop_policy());
        let alice = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
        let mut host = fdae_host_state(storage_provider, alice, Some(policy));

        let own = store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
            .await
            .unwrap();
        assert!(own.is_some(), "alice's own document must be reachable");
        let other = store::Host::get(&mut host, "documents".to_string(), "doc-2".to_string())
            .await
            .unwrap();
        assert!(other.is_none(), "bob's document is unreachable, not an error (ADR-0007)");

        let opts = QueryOptions { filter: None, limit: None, cursor: None };
        let result = store::Host::query(&mut host, "documents".to_string(), opts).await.unwrap();
        let ids: Vec<_> = result.records.iter().map(|r| r.id.clone()).collect();
        assert_eq!(ids, vec!["doc-1"], "bob's document must be excluded from query results");

        assert!(
            store::Host::check_access(
                &mut host,
                "documents".to_string(),
                "doc-1".to_string(),
                Ability::DATA_LAYER_READ.to_string(),
            )
            .await
            .unwrap(),
            "check_access must allow alice's own reachable row"
        );
        assert!(
            !store::Host::check_access(
                &mut host,
                "documents".to_string(),
                "doc-2".to_string(),
                Ability::DATA_LAYER_READ.to_string(),
            )
            .await
            .unwrap(),
            "check_access must deny bob's unreachable row"
        );
    }

    /// CLS: a policy with `fields.deny: ["ssn"]` strips `ssn` from the
    /// payload returned by both `get` and `query` -- the Phase-3 host-side
    /// projection task.md's "CLS: value never returned" row was waiting on.
    #[tokio::test]
    async fn fdae_cls_strips_masked_field_from_get_and_query() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        fdae_seed_documents(storage_provider.clone()).await;

        let policy = Arc::new(fdae_cls_policy());
        let alice = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
        let mut host = fdae_host_state(storage_provider, alice, Some(policy));

        let own = store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
            .await
            .unwrap()
            .unwrap();
        let payload = payload_json(&own);
        assert!(payload.get("ssn").is_none(), "ssn must be stripped from get's payload");
        assert_eq!(payload.get("creator_uuid").and_then(Value::as_str), Some("u-alice"));

        let opts = QueryOptions { filter: None, limit: None, cursor: None };
        let result = store::Host::query(&mut host, "documents".to_string(), opts).await.unwrap();
        assert_eq!(result.records.len(), 1);
        let payload = payload_json(&result.records[0]);
        assert!(payload.get("ssn").is_none(), "ssn must be stripped from query's payload");
    }

    /// Pass-through: `fdae_policy: None` leaves rows and payloads unchanged
    /// -- zero behavior change on the unconfigured (today's production)
    /// path.
    #[tokio::test]
    async fn fdae_policy_absent_is_unfiltered_pass_through() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        fdae_seed_documents(storage_provider.clone()).await;

        let caller = CallerContext::service_system(FDAE_SERVICE_ID);
        let mut host = fdae_host_state(storage_provider, caller, None);

        let opts = QueryOptions { filter: None, limit: None, cursor: None };
        let result = store::Host::query(&mut host, "documents".to_string(), opts).await.unwrap();
        assert_eq!(result.records.len(), 2, "no policy means both rows are visible");
        for record in &result.records {
            assert!(
                payload_json(record).get("ssn").is_some(),
                "no policy means no CLS strip -- ssn must survive untouched"
            );
        }
    }

    /// Lifecycle-hook reads (`init`/`migrate`, which run as
    /// `CallerContext::local_elevated`) must stay unfiltered even under a
    /// deployed policy. Without `query_auth`'s `LocalElevated` exemption,
    /// `local_elevated`'s `data-layer/admin` capability entails
    /// `data-layer/read` and covers every collection, so `compile_read`
    /// compiles a *real* sieve here -- bound to
    /// `"system:local-elevated:<service_id>"`, a DID no principal row can
    /// ever hold -- and both documents would silently vanish. A migration
    /// that reads its own data to decide how to rewrite it would act on
    /// that emptiness instead of erroring.
    #[tokio::test]
    async fn fdae_local_elevated_lifecycle_reads_stay_unfiltered_under_a_policy() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        fdae_seed_documents(storage_provider.clone()).await;

        let policy = Arc::new(fdae_single_hop_policy());
        let caller = CallerContext::local_elevated(FDAE_SERVICE_ID);
        let mut host = fdae_host_state(storage_provider, caller, Some(policy));

        let opts = QueryOptions { filter: None, limit: None, cursor: None };
        let result = store::Host::query(&mut host, "documents".to_string(), opts).await.unwrap();
        assert_eq!(
            result.records.len(),
            2,
            "a lifecycle hook must see every row regardless of the deployed policy"
        );

        let doc = store::Host::get(&mut host, "documents".to_string(), "doc-2".to_string())
            .await
            .unwrap();
        assert!(
            doc.is_some(),
            "get during init/migrate must not be sieved against the synthesized local-elevated \
             identity"
        );
    }

    /// `aggregate` is row-filtered through the host layer identically to
    /// `get`/`query` -- covers the `store::Host::aggregate` wiring seam this
    /// phase adds, which no host test previously exercised with a real
    /// `Some(policy)` (a dropped or `None`-replaced `query_auth()` call here
    /// would have passed every prior test).
    #[tokio::test]
    async fn fdae_aggregate_is_row_filtered_through_host() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        fdae_seed_documents(storage_provider.clone()).await;

        let policy = Arc::new(fdae_single_hop_policy());
        let alice = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
        let mut host = fdae_host_state(storage_provider, alice, Some(policy));

        let result = store::Host::aggregate(
            &mut host,
            "documents".to_string(),
            r#"{"$group":{"_id":null,"n":{"$sum":1}}}"#.to_string(),
        )
        .await
        .unwrap();
        // `SqlValue` doesn't derive `PartialEq` -- compare via its
        // already-derived `Serialize` impl.
        assert_eq!(
            serde_json::to_value(&result.rows).unwrap(),
            serde_json::to_value(vec![vec![SqlValue::Integer(1)]]).unwrap(),
            "only alice's own doc-1 is counted"
        );
    }

    /// `delete_many` is filtered as a write operation through the host layer
    /// -- same wiring-seam coverage gap as `aggregate` above.
    #[tokio::test]
    async fn fdae_delete_many_is_write_filtered_through_host() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        fdae_seed_documents(storage_provider.clone()).await;
        let policy = Arc::new(fdae_write_policy());

        // A read-only capability must not satisfy the write-mode sieve.
        let alice_read_only = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
        let mut host_ro =
            fdae_host_state(storage_provider.clone(), alice_read_only, Some(policy.clone()));
        let deleted =
            store::Host::delete_many(&mut host_ro, "documents".to_string(), String::new())
                .await
                .unwrap();
        assert_eq!(deleted, 0, "a read-only capability must not delete anything");

        // A write capability deletes only alice's own row.
        let alice_write = fdae_caller("did:key:alice", vec![fdae_write_cap("documents")]);
        let mut host_rw = fdae_host_state(storage_provider, alice_write, Some(policy));
        let deleted =
            store::Host::delete_many(&mut host_rw, "documents".to_string(), String::new())
                .await
                .unwrap();
        assert_eq!(deleted, 1, "only alice's own document is deletable");
    }

    /// Through the `store::Host` guest boundary: a
    /// write-capable caller who cannot reach a row via the compiled sieve
    /// is denied `put`/`patch`/`delete` on it; the same caller against a
    /// row they do reach succeeds.
    #[tokio::test]
    async fn fdae_put_patch_delete_deny_an_unreachable_row_and_allow_a_reachable_one() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        fdae_seed_documents(storage_provider.clone()).await;
        let policy = Arc::new(fdae_write_policy());
        let alice = fdae_caller("did:key:alice", vec![fdae_write_cap("documents")]);
        let mut host = fdae_host_state(storage_provider, alice, Some(policy));

        // doc-2 belongs to bob -- unreachable to alice under the write sieve.
        let err = store::Host::patch(
            &mut host,
            "documents".to_string(),
            "doc-2".to_string(),
            br#"{"x":1}"#.to_vec(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DataLayerError::PermissionDenied));

        let err = store::Host::delete(&mut host, "documents".to_string(), "doc-2".to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, DataLayerError::PermissionDenied));

        let err = store::Host::put(
            &mut host,
            "documents".to_string(),
            RecordWriteValue {
                id: "doc-2".to_string(),
                payload: json!({"creator_uuid": "u-bob", "hijacked": true})
                    .to_string()
                    .into_bytes(),
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DataLayerError::PermissionDenied));

        // doc-1 belongs to alice -- reachable.
        store::Host::patch(
            &mut host,
            "documents".to_string(),
            "doc-1".to_string(),
            br#"{"nickname":"al"}"#.to_vec(),
        )
        .await
        .unwrap();
        let record = store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(payload_json(&record)["nickname"], "al");

        store::Host::delete(&mut host, "documents".to_string(), "doc-1".to_string()).await.unwrap();
        assert!(
            store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
                .await
                .unwrap()
                .is_none()
        );
    }

    /// `drop_collection` bypasses any per-row policy on the collection
    /// entirely, so it must not be reachable through an ordinary write
    /// capability: a caller holding only `data-layer/write` on `documents`
    /// (able to `put`/`patch`/`delete` rows it can individually reach) is
    /// denied `drop_collection("documents")` outright; a caller holding
    /// `data-layer/admin` on the service succeeds.
    #[tokio::test]
    async fn drop_collection_requires_admin_not_an_ordinary_write_capability() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        fdae_seed_documents(storage_provider.clone()).await;

        let writer = fdae_caller("did:key:alice", vec![fdae_write_cap("documents")]);
        let mut host = fdae_host_state(storage_provider.clone(), writer, None);
        let err =
            store::Host::drop_collection(&mut host, "documents".to_string()).await.unwrap_err();
        assert!(matches!(err, DataLayerError::PermissionDenied));
        assert!(
            store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
                .await
                .unwrap()
                .is_some(),
            "a denied drop_collection must leave the collection intact"
        );

        let admin = fdae_caller(
            "did:key:admin",
            vec![Capability {
                with: ResourceUri::service(FDAE_SERVICE_ID, FDAE_SERVICE_ID),
                can: Ability(Ability::DATA_LAYER_ADMIN.to_string()),
                caveats: None,
            }],
        );
        let mut host = fdae_host_state(storage_provider, admin, None);
        store::Host::drop_collection(&mut host, "documents".to_string()).await.unwrap();
    }

    /// **D-04-02-g CLS-narrowing pin** (task.md Decision Register): the same
    /// "an extra capability shouldn't narrow" defect that Phase 2 pinned for
    /// RLS (`tests_fdae.rs::two_capabilities_with_conflicting_caveats_
    /// currently_narrow_to_zero_rows`) applies to CLS `fields.deny` union
    /// across capabilities too, and only becomes observable now that
    /// field-stripping ships (Phase 3). Alice holds both an unrestricted
    /// `read` capability and a second `read` capability caveated
    /// `fields.deny: ["ssn"]` on the same resource; today's `compile_cls`
    /// unions every entitling capability's deny-list, so even the
    /// unrestricted grant's payload comes back stripped. If D-04-02-g is
    /// fixed, this assertion should flip to `ssn` being **present** (the
    /// unrestricted capability's caveat-free access should win).
    #[tokio::test]
    async fn fdae_d04_02_g_extra_caveated_capability_narrows_cls_strip() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        fdae_seed_documents(storage_provider.clone()).await;

        // `fdae_single_hop_policy` carries no policy-level `fields.deny` --
        // the mask below comes entirely from the second capability's caveat.
        let policy = Arc::new(fdae_single_hop_policy());
        let unrestricted_cap = fdae_read_cap("documents");
        let ssn_deny_cap = Capability {
            with: fdae_resource("documents"),
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: Some(json!({"fields": {"deny": ["ssn"]}})),
        };
        let alice = fdae_caller("did:key:alice", vec![unrestricted_cap, ssn_deny_cap]);
        let mut host = fdae_host_state(storage_provider, alice, Some(policy));

        let own = store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
            .await
            .unwrap()
            .unwrap();
        let payload = payload_json(&own);
        assert!(
            payload.get("ssn").is_none(),
            "D-04-02-g: today, the caveated capability's fields.deny narrows the unrestricted \
             capability's access too, so ssn is stripped even though the unrestricted grant alone \
             should expose it. If this assertion starts failing, D-04-02-g has been fixed -- \
             update this test to assert ssn IS present."
        );
    }

    // -- Slice B3 Phase 4: cross-service relationship-proof fetch, wired
    // through `HostState::resolve_query_auth` --------------------------

    fn fdae_remote_relation_policy(expected_asserter_did: &str) -> Policy {
        parse_and_validate(&format!(
            r#"{{
                "version": "fdae/v1",
                "definitions": {{
                    "document": {{
                        "table": "documents",
                        "relations": {{"owner": {{
                            "target": "employee", "service": "hr-svc",
                            "join_column": "owner_uuid",
                            "expected_asserter_did": "{expected_asserter_did}"
                        }}}},
                        "permissions": {{
                            "view": {{"allows": ["data-layer/read"], "paths": [["owner", "anchor"]]}}
                        }}
                    }}
                }}
            }}"#
        ))
        .unwrap()
    }

    #[derive(Debug)]
    struct StubProxy(Mutex<Option<Result<Value, RpcProxyError>>>);

    #[async_trait::async_trait]
    impl ServiceProxy for StubProxy {
        async fn invoke(&self, _request: ProxyRequest) -> Result<Value, RpcProxyError> {
            self.0.lock().unwrap().take().expect("StubProxy invoked with no response configured")
        }
    }

    async fn seed_one_remote_owned_document(storage_provider: Arc<dyn StorageProvider>) {
        let mut seeder =
            fdae_host_state(storage_provider, CallerContext::service_system(FDAE_SERVICE_ID), None);
        store::Host::create_collection(
            &mut seeder,
            CollectionSchema { name: "documents".to_string(), indexes: vec![] },
        )
        .await
        .unwrap();
        store::Host::put(
            &mut seeder,
            "documents".to_string(),
            RecordWriteValue {
                id: "doc-1".to_string(),
                payload: json!({"owner_uuid": "emp-alice"}).to_string().into_bytes(),
            },
        )
        .await
        .unwrap();
    }

    /// A policy naming a remote relation resolves through `resolve_query_auth`:
    /// `get` reaches `HostState.service_proxy`, verifies the returned
    /// `RelationshipProof` against the policy's `expected_asserter_did`, and
    /// the finalized sieve correctly admits alice's own document.
    #[tokio::test]
    async fn fdae_remote_relation_fetch_succeeds_through_host_state() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        seed_one_remote_owned_document(storage_provider.clone()).await;

        let identity = syneroym_identity::Identity::generate().unwrap();
        let asserter_did = substrate::derive_did_key(&identity.public_key());
        let proof = syneroym_rpc::RelationshipProof::sign(
            &identity,
            None,
            "employee",
            "did:key:alice",
            vec!["emp-alice".to_string()],
        )
        .unwrap();
        let stub: Arc<dyn ServiceProxy> =
            Arc::new(StubProxy(Mutex::new(Some(Ok(serde_json::to_value(&proof).unwrap())))));

        let policy = Arc::new(fdae_remote_relation_policy(&asserter_did));
        let alice = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
        let mut host = HostState::new(
            FDAE_SERVICE_ID.to_string(),
            None,
            Arc::new(KeyStore::new()),
            storage_provider,
            test_blob_provider(),
            alice,
            0,
            test_messaging_context(),
            test_streaming_context(),
            Arc::downgrade(&stub),
            Some(policy),
            false,
            syneroym_rpc::empty_row_authorizer(),
            None,
            syneroym_app_orchestration::empty_resolver(),
        );

        let own = store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
            .await
            .unwrap();
        assert!(
            own.is_some(),
            "alice's document must resolve through the real cross-service fetch"
        );
    }

    /// A fetch failure (the remote proxy call errors) denies the whole read
    /// closed rather than falling back to unfiltered or silently empty --
    /// `get` must surface an `Err`, not `Ok(None)` masquerading as "not
    /// found."
    #[tokio::test]
    async fn fdae_remote_relation_fetch_failure_denies_closed() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        seed_one_remote_owned_document(storage_provider.clone()).await;

        let stub: Arc<dyn ServiceProxy> = Arc::new(StubProxy(Mutex::new(Some(Err(
            RpcProxyError::Timeout(Duration::from_secs(5)),
        )))));

        let policy = Arc::new(fdae_remote_relation_policy("did:key:zSomeAsserter"));
        let alice = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
        let mut host = HostState::new(
            FDAE_SERVICE_ID.to_string(),
            None,
            Arc::new(KeyStore::new()),
            storage_provider,
            test_blob_provider(),
            alice,
            0,
            test_messaging_context(),
            test_streaming_context(),
            Arc::downgrade(&stub),
            Some(policy),
            false,
            syneroym_rpc::empty_row_authorizer(),
            None,
            syneroym_app_orchestration::empty_resolver(),
        );

        let err = store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, DataLayerError::PermissionDenied));
    }

    // -- sagas -----------------------------------------------------------

    fn saga_host(
        read_only: bool,
        proxy: &Arc<RecordingProxy>,
        db_dir: &std::path::Path,
    ) -> HostState {
        HostState::new(
            "driver".to_string(),
            None,
            Arc::new(KeyStore::new()),
            Arc::new(SqliteStorageProvider::new(db_dir, false).unwrap()),
            test_blob_provider(),
            CallerContext::service_system("driver"),
            0,
            test_messaging_context(),
            test_streaming_context(),
            Arc::downgrade(proxy) as Weak<dyn ServiceProxy>,
            None,
            read_only,
            syneroym_rpc::empty_row_authorizer(),
            None,
            syneroym_app_orchestration::empty_resolver(),
        )
    }

    #[tokio::test]
    async fn begin_reaches_the_fake_proxy_with_the_fields_the_wit_carried() {
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = saga_host(false, &proxy, temp_dir.path());

        let saga_id =
            saga::Host::begin(&mut host, "checkout".to_string(), Some(120)).await.unwrap();
        assert_eq!(saga_id, "saga-1");

        let recorded = proxy.last_saga_begin.lock().unwrap().clone().unwrap();
        assert_eq!(recorded.caller_service_id, "driver");
        assert_eq!(recorded.name, "checkout");
        assert_eq!(recorded.deadline_secs, Some(120));
    }

    #[tokio::test]
    async fn step_reaches_the_fake_proxy_with_the_fields_the_wit_carried() {
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = saga_host(false, &proxy, temp_dir.path());

        let result = saga::Host::step(
            &mut host,
            "saga-1".to_string(),
            CallTarget::Service("did:key:zParticipant".to_string()),
            "saga-participant".to_string(),
            "reserve".to_string(),
            "{\"item\":\"a\"}".to_string(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(result, "null");

        let recorded = proxy.last_saga_step.lock().unwrap().clone().unwrap();
        assert_eq!(recorded.saga_id, "saga-1");
        assert_eq!(recorded.target, QueuedTarget::Service("did:key:zParticipant".to_string()));
        assert_eq!(recorded.interface, "saga-participant");
        assert_eq!(recorded.method, "reserve");
        assert_eq!(recorded.params, serde_json::json!({"item": "a"}));
    }

    #[tokio::test]
    async fn commit_reaches_the_fake_proxy_with_the_saga_id() {
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = saga_host(false, &proxy, temp_dir.path());

        saga::Host::commit(&mut host, "saga-1".to_string()).await.unwrap();

        let recorded = proxy.last_saga_commit.lock().unwrap().clone().unwrap();
        assert_eq!(recorded, ("driver".to_string(), "saga-1".to_string()));
    }

    #[tokio::test]
    async fn compensate_reaches_the_fake_proxy_with_the_saga_id() {
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = saga_host(false, &proxy, temp_dir.path());

        saga::Host::compensate(&mut host, "saga-1".to_string()).await.unwrap();

        let recorded = proxy.last_saga_compensate.lock().unwrap().clone().unwrap();
        assert_eq!(recorded, ("driver".to_string(), "saga-1".to_string()));
    }

    #[tokio::test]
    async fn status_maps_the_fake_proxys_answer_onto_the_wit_record() {
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = saga_host(false, &proxy, temp_dir.path());

        let status = saga::Host::status(&mut host, "saga-1".to_string()).await.unwrap();
        assert_eq!(status.saga_id, "saga-1");
        assert_eq!(status.name, "wf");
        assert_eq!(status.state, WitSagaState::Open);
        assert_eq!(status.steps, 1);
        assert_eq!(status.compensated_steps, 0);
        assert_eq!(status.created_at, 1_000);
        assert_eq!(status.deadline_at, 4_600_000);
        assert!(status.last_error.is_none());
    }

    /// ADR-0017 §7 is *local* read-only lookups; a stage-4 after-step
    /// instance may not originate any proxy call, saga included -- the
    /// same refusal `proxy::Host::call`/`enqueue` apply.
    #[tokio::test]
    async fn a_stage_four_after_step_instance_cannot_open_a_saga() {
        let proxy = Arc::new(RecordingProxy::default());
        let temp_dir = tempfile::tempdir().unwrap();
        let mut host = saga_host(true, &proxy, temp_dir.path());

        let err = saga::Host::begin(&mut host, "checkout".to_string(), None).await.unwrap_err();
        assert!(
            matches!(err, proxy::ProxyError::Internal(_)),
            "expected the stage-4 refusal, got {err:?}"
        );
        assert!(
            proxy.last_saga_begin.lock().unwrap().is_none(),
            "the fake proxy must not be reached"
        );
    }
}
