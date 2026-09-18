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
use syneroym_app_host::types::http::FrameKind as AppFrameKind;
use syneroym_app_orchestration::{AppInstanceId, LogicalResolver, LogicalServiceName, TopologyKey};
use syneroym_core::{
    local_registry::SubstrateEndpoint,
    record_signer::{
        CallerBinding, NodeRecordSigner, SigningError, SigningIdentity, SigningPrincipal,
    },
};
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
    ConversationError as RpcConversationError, ProxyError as RpcProxyError, ProxyProtocol,
    ProxyRequest, QueuedCall, QueuedTarget, ResourceUri, RowAuthorizer, SagaBegin,
    SagaState as RpcSagaState, SagaStepRequest, ServiceProxy, WebSocketSenders, apply_stage4,
    union_masked_fields,
};
use syneroym_wit_interfaces::{
    conversation_host::syneroym::conversation::conversation as wit_conversation,
    host::syneroym::{
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
    },
    http_host::syneroym::http::{websocket, websocket_types::FrameKind as WitFrameKind},
    invocation_host::syneroym::invocation::invocation::{
        self as invocation, CallerOrigin as WitCallerOrigin,
    },
    signing_host::syneroym::signing::signing::{
        self, Principal as WitPrincipal, RecordDraft as WitRecordDraft,
        SigningError as WitSigningError, SigningIdentity as WitSigningIdentity,
    },
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

pub(super) fn map_broker_error(e: BrokerMessagingError) -> MessagingError {
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

/// Marker type for [`empty_conversation_host`]'s always-empty `Weak`: never
/// constructed (only its type is used, via unsized coercion), so every
/// trait method below is unreachable by construction, not merely by
/// convention.
#[derive(Debug)]
struct NeverConstructedConversationHost;

#[async_trait::async_trait]
impl syneroym_rpc::ConversationHost for NeverConstructedConversationHost {
    async fn open_direct(
        &self,
        _: &str,
        _: &str,
    ) -> Result<String, syneroym_rpc::ConversationError> {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
    async fn conversations(
        &self,
        _: &str,
    ) -> Result<Vec<syneroym_rpc::ConversationSummary>, syneroym_rpc::ConversationError> {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
    async fn send(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: Vec<u8>,
    ) -> Result<String, syneroym_rpc::ConversationError> {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
    async fn history(
        &self,
        _: &str,
        _: &str,
        _: u32,
        _: Option<String>,
    ) -> Result<syneroym_rpc::ConversationHistoryPage, syneroym_rpc::ConversationError> {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
    async fn delivery_status(
        &self,
        _: &str,
        _: &str,
    ) -> Result<syneroym_rpc::ConversationDeliveryState, syneroym_rpc::ConversationError> {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
    async fn outbox(
        &self,
        _: &str,
    ) -> Result<Vec<syneroym_rpc::ConversationMessage>, syneroym_rpc::ConversationError> {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
    async fn retry(&self, _: &str, _: &str) -> Result<(), syneroym_rpc::ConversationError> {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
    async fn create_group(&self, _: &str) -> Result<String, syneroym_rpc::ConversationError> {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
    async fn add_member(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), syneroym_rpc::ConversationError> {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
    async fn remove_member(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), syneroym_rpc::ConversationError> {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
    async fn members(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Vec<String>, syneroym_rpc::ConversationError> {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
    async fn membership_history(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Vec<syneroym_rpc::ConversationMembershipEvent>, syneroym_rpc::ConversationError>
    {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
    async fn sync_now(&self, _: &str, _: &str) -> Result<(), syneroym_rpc::ConversationError> {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
    async fn group_push(
        &self,
        _: &str,
        _: &str,
        _: Vec<u8>,
    ) -> Result<Vec<u8>, syneroym_rpc::ConversationError> {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
    async fn group_sync(
        &self,
        _: &str,
        _: &str,
        _: Vec<u8>,
    ) -> Result<Vec<u8>, syneroym_rpc::ConversationError> {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
    async fn prekey_bundle(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Vec<u8>, syneroym_rpc::ConversationError> {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
    async fn peer_deliver(
        &self,
        _: &str,
        _: &str,
        _: Vec<u8>,
    ) -> Result<Vec<u8>, syneroym_rpc::ConversationError> {
        unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
    }
}

/// An always-empty `Weak<dyn ConversationHost>` -- mirrors
/// [`empty_service_proxy`] exactly, for `HostState.conversation`'s default
/// before [`HostState::with_conversation`] sets a real one.
pub(crate) fn empty_conversation_host() -> Weak<dyn syneroym_rpc::ConversationHost> {
    Weak::<NeverConstructedConversationHost>::new()
}

/// Host state instantiated per-request for WASM components
/// Where this invocation entered the node, which the caller alone cannot
/// say: a sibling's proxy call and an unauthenticated inbound stream both
/// arrive with the synthesized `service_system` caller, so the two are
/// indistinguishable from `caller` by construction. `Local` is every
/// host-driven path -- lifecycle hooks, `notify_guest_message`, the
/// stage-4 after-step, a guest-to-guest proxy call, and a test harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InvocationOrigin {
    /// A local dispatch path. There is no attributable identity, and none
    /// is claimed.
    #[default]
    Local,
    /// The router's inbound JSON-RPC dispatch off the wire. The only
    /// producer is `AppSandboxEngine::execute_wasm_json_from_wire`.
    Wire,
}

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
    /// Where this invocation entered the node. Defaults to
    /// [`InvocationOrigin::Local`]; set to `Wire` only by
    /// [`AppSandboxEngine::execute_wasm_json_from_wire`]. The `caller`
    /// alone cannot carry this -- see the enum's own doc comment.
    pub invocation_origin: InvocationOrigin,
    /// Compiled FDAE policy for this service, loaded at instantiation by
    /// `AppSandboxEngine::resolve_fdae_policy`, or `None` when the service
    /// has no policy (the unfiltered default).
    pub fdae_policy: Option<Arc<Policy>>,
    pub config_generation: u64,
    pub messaging: MessagingContext,
    pub streaming: StreamContext,
    /// Weak handle to the Universal Proxy, letting a guest
    /// originate a cross-service call via `syneroym:proxy/proxy::call`.
    /// `Weak`, not `Arc`: `ProxyRouter` (the only implementation) itself
    /// holds a `Weak<AppSandboxEngine>` back for local WASM targets, so two
    /// strong refs would form the same class of uncollectable cycle that
    /// once hung graceful shutdown.
    pub service_proxy: Weak<dyn ServiceProxy>,
    /// Stage-4 after-step instances (`AuthLevel::LocalReadOnly`) get this
    /// set: every mutating and egress host function hard-denies. Not
    /// derivable from `caller.auth` alone -- write host paths carry no
    /// capability gate today, so the check has to live somewhere
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
    /// Weak handle to the Conversation service. Defaults to an always-empty
    /// `Weak::new()` — `HostState::new`'s signature does not change; set via
    /// [`Self::with_conversation`] at the two real construction sites only.
    pub conversation: Weak<dyn syneroym_rpc::ConversationHost>,
    /// Live WebSocket connections table shared across components and the host.
    pub websocket_senders: Arc<syneroym_rpc::WebSocketSenders>,
    /// The node's record signer (`syneroym:signing`). `Option`, not
    /// `Weak`: it holds no path back to this engine, so there is no cycle,
    /// and `None` is the honest state for a node that never wired one
    /// (every existing `HostState::new` call site, all of them tests).
    pub record_signer: Option<Arc<NodeRecordSigner>>,
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
            invocation_origin: InvocationOrigin::Local,
            fdae_policy,
            config_generation,
            messaging,
            streaming,
            service_proxy,
            read_only,
            row_authorizer,
            app_instance_id,
            logical_resolver,
            conversation: empty_conversation_host(),
            websocket_senders: WebSocketSenders::new(),
            record_signer: None,
        }
    }

    /// Sets [`Self::conversation`] after construction — the two real
    /// construction sites (`AppSandboxEngine::instantiate`-adjacent
    /// code) call this; every other `HostState::new` call site is
    /// unaffected.
    #[must_use]
    pub fn with_conversation(
        mut self,
        conversation: Weak<dyn syneroym_rpc::ConversationHost>,
    ) -> Self {
        self.conversation = conversation;
        self
    }

    #[must_use]
    pub fn with_record_signer(mut self, signer: Option<Arc<NodeRecordSigner>>) -> Self {
        self.record_signer = signer;
        self
    }

    /// Sets [`Self::invocation_origin`] after construction. Only
    /// [`AppSandboxEngine::execute_wasm_json_from_wire`] passes `Wire`;
    /// every other path keeps the `Local` default.
    #[must_use]
    pub fn with_invocation_origin(mut self, origin: InvocationOrigin) -> Self {
        self.invocation_origin = origin;
        self
    }

    /// Sets [`Self::websocket_senders`] after construction.
    #[must_use]
    pub fn with_websocket_senders(
        mut self,
        websocket_senders: Arc<syneroym_rpc::WebSocketSenders>,
    ) -> Self {
        self.websocket_senders = websocket_senders;
        self
    }

    /// Builds the `QueryAuth` for the current request from `fdae_policy` +
    /// `caller.session`, or `None` on the policy-absent path (today's
    /// unfiltered behavior). Runs `syneroym_fdae::
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
    /// distinct reason (ADR-0017 §7).** It is the stage-4
    /// after-step's own identity: the ADR is explicit that the after-step's
    /// optional lookups read this service's data unfiltered -- the service
    /// owner authored the policy and could equally have written the same
    /// call into their service code, and running under the caller's
    /// authority breaks most real policies. Read-only-ness comes from
    /// `HostState.read_only` (hard-denying every mutating/egress host
    /// function), not from the sieve. **This early return is also what
    /// bounds after-step recursion**: an after-step instance's own
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

mod capabilities_blob;
mod capabilities_messaging;
mod capabilities_proxy;
#[cfg(test)]
use capabilities_proxy::MAX_IDEMPOTENCY_KEY_BYTES;
mod capabilities_services;
mod capabilities_store;

#[cfg(test)]
pub(crate) mod tests;
