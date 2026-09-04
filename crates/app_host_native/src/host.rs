//! [`NativeAppHost`]: one invocation's handle to the host, and the trait
//! impls that delegate every call to `HostState`'s existing `Host` impls --
//! the same semantics the WASM build reaches, through the same code.

use std::{fmt, sync::Arc};

use syneroym_app_host::{
    AppAppConfig, AppBlobReader, AppBlobStore, AppBlobWriter, AppConversation, AppDataLayer,
    AppInvocation, AppMessaging, AppProxy, AppSigning, AppVault, AppWebSocket,
    types::{
        app_config::ConfigError,
        blob_store::BlobError,
        conversation::{
            ConversationError, ConversationSummary, DeliveryState, HistoryPage, MembershipEvent,
            Message,
        },
        data_layer::{
            CollectionSchema, DataLayerError, Mutation, QueryOptions, QueryResult, RawQueryResult,
            RecordReadValue, RecordWriteValue, SqlValue,
        },
        http::FrameKind,
        invocation::CallerOrigin,
        messaging::MessagingError,
        proxy::{CallOptions, CallTarget, ProxyError},
        signing::{Principal, RecordDraft, SigningError, SigningIdentity},
        vault::VaultError,
    },
};
use syneroym_rpc::{AuthLevel, CallerContext};
use syneroym_sandbox_wasm::{HostState, InvocationOrigin};
// Aliased: `data_layer::store`, `blob_store::blob_store`, `messaging::host_api`,
// `proxy::proxy`, `app_config::app_config`, `vault::vault` each define their own `Host` trait.
use syneroym_wit_interfaces::conversation_host::syneroym::conversation::conversation::Host as HostConversation;
use syneroym_wit_interfaces::{
    host::syneroym::{
        app_config::app_config::Host as HostAppConfig,
        blob_store::blob_store::{Host as HostBlobStore, HostBlobReader, HostBlobWriter},
        data_layer::store::Host as HostStore,
        messaging::host_api::Host as HostMessaging,
        proxy::proxy::Host as HostProxy,
        vault::vault::Host as HostVault,
    },
    signing_host::syneroym::signing::signing::Host as HostSigning,
};
use tokio::sync::{Mutex, OnceCell};
use wasmtime::component::Resource;

use crate::{convert, factory::NativeHostFactory};

/// One invocation's handle to the host. A cheap `Arc` newtype, so a blob
/// resource returned by `open_upload` can hold its host without borrowing
/// it -- which is what keeps the trait methods on `&self` and keeps a
/// lifetime out of `AppBlobStore::Writer`.
#[derive(Debug, Clone)]
pub struct NativeAppHost(Arc<HostInner>);

impl NativeAppHost {
    pub(crate) fn new(inner: Arc<HostInner>) -> Self {
        Self(inner)
    }
}

/// `derive` works here only because `NativeHostFactory` has its own manual
/// `Debug` impl and `Mutex<T>: Debug where T: Debug` (`HostState`
/// supplies its own).
#[derive(Debug)]
pub(crate) struct HostInner {
    pub(crate) factory: Arc<NativeHostFactory>,
    pub(crate) caller: CallerContext,
    pub(crate) read_only: bool,
    /// Where this call entered the node. The factory sets it: `host_for`
    /// means local, `host_for_wire` means wire. The native shim has no
    /// per-invocation origin to read (`NativeInvocation` carries none), so
    /// it is carried by which constructor the factory called instead.
    pub(crate) invocation_origin: InvocationOrigin,
    /// Lazy: instantiated on the first host call that needs a `HostState`.
    /// Methods that do not need one (`subscribe`/`unsubscribe`, outbound
    /// `send_websocket_frame`) never initialize this.
    pub(crate) state: OnceCell<Mutex<HostState>>,
}

impl HostInner {
    pub(crate) async fn state_mutex(&self) -> &Mutex<HostState> {
        self.state
            .get_or_init(|| async {
                Mutex::new(self.factory.build_host_state(self.caller.clone(), self.read_only).await)
            })
            .await
    }
}

impl AppDataLayer for NativeAppHost {
    async fn create_collection(&self, schema: CollectionSchema) -> Result<(), DataLayerError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostStore::create_collection(&mut *state, convert::collection_schema_in(schema))
            .await
            .map_err(convert::data_layer_error_out)
    }

    async fn drop_collection(&self, name: String) -> Result<(), DataLayerError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostStore::drop_collection(&mut *state, name).await.map_err(convert::data_layer_error_out)
    }

    async fn put(&self, collection: String, value: RecordWriteValue) -> Result<(), DataLayerError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostStore::put(&mut *state, collection, convert::record_write_value_in(value))
            .await
            .map_err(convert::data_layer_error_out)
    }

    async fn patch(
        &self,
        collection: String,
        id: String,
        patch_json: Vec<u8>,
    ) -> Result<(), DataLayerError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostStore::patch(&mut *state, collection, id, patch_json)
            .await
            .map_err(convert::data_layer_error_out)
    }

    async fn get(
        &self,
        collection: String,
        id: String,
    ) -> Result<Option<RecordReadValue>, DataLayerError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostStore::get(&mut *state, collection, id)
            .await
            .map(|opt| opt.map(convert::record_read_value_out))
            .map_err(convert::data_layer_error_out)
    }

    async fn query(
        &self,
        collection: String,
        opts: QueryOptions,
    ) -> Result<QueryResult, DataLayerError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostStore::query(&mut *state, collection, convert::query_options_in(opts))
            .await
            .map(convert::query_result_out)
            .map_err(convert::data_layer_error_out)
    }

    async fn aggregate(
        &self,
        collection: String,
        pipeline: String,
    ) -> Result<RawQueryResult, DataLayerError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostStore::aggregate(&mut *state, collection, pipeline)
            .await
            .map(convert::raw_query_result_out)
            .map_err(convert::data_layer_error_out)
    }

    async fn delete(&self, collection: String, id: String) -> Result<(), DataLayerError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostStore::delete(&mut *state, collection, id).await.map_err(convert::data_layer_error_out)
    }

    async fn delete_many(&self, collection: String, filter: String) -> Result<u64, DataLayerError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostStore::delete_many(&mut *state, collection, filter)
            .await
            .map_err(convert::data_layer_error_out)
    }

    async fn batch_mutate(
        &self,
        collection: String,
        mutations: Vec<Mutation>,
    ) -> Result<(), DataLayerError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostStore::batch_mutate(
            &mut *state,
            collection,
            mutations.into_iter().map(convert::mutation_in).collect(),
        )
        .await
        .map_err(convert::data_layer_error_out)
    }

    async fn execute_ddl(&self, sql: String) -> Result<(), DataLayerError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostStore::execute_ddl(&mut *state, sql).await.map_err(convert::data_layer_error_out)
    }

    async fn query_raw(
        &self,
        sql: String,
        params: Vec<SqlValue>,
    ) -> Result<RawQueryResult, DataLayerError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostStore::query_raw(
            &mut *state,
            sql,
            params.into_iter().map(convert::sql_value_in).collect(),
        )
        .await
        .map(convert::raw_query_result_out)
        .map_err(convert::data_layer_error_out)
    }

    async fn check_access(
        &self,
        collection: String,
        id: String,
        operation: String,
    ) -> Result<bool, DataLayerError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostStore::check_access(&mut *state, collection, id, operation)
            .await
            .map_err(convert::data_layer_error_out)
    }
}

impl AppBlobStore for NativeAppHost {
    type Writer = NativeBlobWriter;
    type Reader = NativeBlobReader;

    async fn put_blob(&self, data: Vec<u8>) -> Result<String, BlobError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostBlobStore::put_blob(&mut *state, data).await.map_err(convert::blob_error_out)
    }

    async fn get_blob(&self, hash: String) -> Result<Vec<u8>, BlobError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostBlobStore::get_blob(&mut *state, hash).await.map_err(convert::blob_error_out)
    }

    async fn open_upload(&self) -> Result<NativeBlobWriter, BlobError> {
        let rep = {
            let mut state = self.0.state_mutex().await.lock().await;
            HostBlobStore::open_upload(&mut *state).await.map_err(convert::blob_error_out)?.rep()
        };
        Ok(NativeBlobWriter { host: self.clone(), rep })
    }

    async fn open_download(
        &self,
        hash: String,
        offset: u64,
    ) -> Result<NativeBlobReader, BlobError> {
        let rep = {
            let mut state = self.0.state_mutex().await.lock().await;
            HostBlobStore::open_download(&mut *state, hash, offset)
                .await
                .map_err(convert::blob_error_out)?
                .rep()
        };
        Ok(NativeBlobReader { host: self.clone(), rep })
    }

    async fn delete_blob(&self, hash: String) -> Result<(), BlobError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostBlobStore::delete_blob(&mut *state, hash).await.map_err(convert::blob_error_out)
    }

    async fn signed_url(&self, hash: String, ttl_secs: u32) -> Result<String, BlobError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostBlobStore::signed_url(&mut *state, hash, ttl_secs)
            .await
            .map_err(convert::blob_error_out)
    }
}

/// The `rep` (rather than a stored `Resource<T>`) is deliberate: the host
/// methods take the resource handle by value, and rebuilding
/// `Resource::new_own(rep)` per call is what `HostBlobWriter`'s own
/// implementation expects -- the table, not the handle, owns the session's
/// lifetime, and `rep()` is all any of `write`/`finish`/`abort` ever read
/// back out of it. Deliberately no `Drop` here: if a caller drops this
/// without calling `finish`/`abort`, the table entry (and the boxed
/// `UploadSession` inside it) is torn down whenever the per-invocation
/// `HostState` that owns the table is -- and the quota refund that matters
/// lives in `ObjectStoreUploadSession`'s own `Drop`
/// (`crates/data_blob/src/object_store_impl.rs`), which fires exactly then,
/// synchronously, on both builds. Nothing here needs to race that.
pub struct NativeBlobWriter {
    host: NativeAppHost,
    rep: u32,
}

impl fmt::Debug for NativeBlobWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeBlobWriter").field("rep", &self.rep).finish_non_exhaustive()
    }
}

impl NativeBlobWriter {
    /// Hidden: not part of this crate's public API, just a way for the
    /// dual-build parity suite to observe the table index a fresh
    /// `HostState` handed out, rather than only inferring it indirectly.
    #[doc(hidden)]
    #[must_use]
    pub fn rep(&self) -> u32 {
        self.rep
    }
}

impl AppBlobWriter for NativeBlobWriter {
    async fn write(&mut self, chunk: Vec<u8>) -> Result<(), BlobError> {
        let mut state = self.host.0.state_mutex().await.lock().await;
        HostBlobWriter::write(&mut *state, Resource::new_own(self.rep), chunk)
            .await
            .map_err(convert::blob_error_out)
    }

    async fn finish(self) -> Result<String, BlobError> {
        let mut state = self.host.0.state_mutex().await.lock().await;
        HostBlobWriter::finish(&mut *state, Resource::new_own(self.rep))
            .await
            .map_err(convert::blob_error_out)
    }

    async fn abort(self) {
        let mut state = self.host.0.state_mutex().await.lock().await;
        HostBlobWriter::abort(&mut *state, Resource::new_own(self.rep)).await;
    }
}

pub struct NativeBlobReader {
    host: NativeAppHost,
    rep: u32,
}

impl fmt::Debug for NativeBlobReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeBlobReader").field("rep", &self.rep).finish_non_exhaustive()
    }
}

impl AppBlobReader for NativeBlobReader {
    async fn read(&mut self, max_bytes: u32) -> Result<Vec<u8>, BlobError> {
        let mut state = self.host.0.state_mutex().await.lock().await;
        HostBlobReader::read(&mut *state, Resource::new_own(self.rep), max_bytes)
            .await
            .map_err(convert::blob_error_out)
    }
}

impl AppMessaging for NativeAppHost {
    async fn publish(&self, topic: String, payload: Vec<u8>) -> Result<(), MessagingError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostMessaging::publish(&mut *state, topic, payload).await.map_err(convert::msg_error_out)
    }

    async fn subscribe(&self, topic: String) -> Result<(), MessagingError> {
        if self.0.read_only {
            return Err(MessagingError::PermissionDenied);
        }
        self.0.factory.subscribe(topic).await
    }

    async fn unsubscribe(&self, topic: String) -> Result<(), MessagingError> {
        if self.0.read_only {
            return Err(MessagingError::PermissionDenied);
        }
        self.0.factory.unsubscribe(&topic)
    }
}

impl AppConversation for NativeAppHost {
    async fn open_direct(&self, peer_address: String) -> Result<String, ConversationError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostConversation::open_direct(&mut *state, peer_address)
            .await
            .map_err(convert::conversation_error_out)
    }

    async fn conversations(&self) -> Result<Vec<ConversationSummary>, ConversationError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostConversation::conversations(&mut *state)
            .await
            .map(|v| v.into_iter().map(convert::conversation_summary_out).collect())
            .map_err(convert::conversation_error_out)
    }

    async fn send(
        &self,
        conversation: String,
        content_type: String,
        body: Vec<u8>,
    ) -> Result<String, ConversationError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostConversation::send(&mut *state, conversation, content_type, body)
            .await
            .map_err(convert::conversation_error_out)
    }

    async fn history(
        &self,
        conversation: String,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<HistoryPage, ConversationError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostConversation::history(&mut *state, conversation, limit, cursor)
            .await
            .map(convert::history_page_out)
            .map_err(convert::conversation_error_out)
    }

    async fn delivery_status(&self, message: String) -> Result<DeliveryState, ConversationError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostConversation::delivery_status(&mut *state, message)
            .await
            .map(convert::delivery_state_out)
            .map_err(convert::conversation_error_out)
    }

    async fn outbox(&self) -> Result<Vec<Message>, ConversationError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostConversation::outbox(&mut *state)
            .await
            .map(|v| v.into_iter().map(convert::message_out).collect())
            .map_err(convert::conversation_error_out)
    }

    async fn retry(&self, message: String) -> Result<(), ConversationError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostConversation::retry(&mut *state, message).await.map_err(convert::conversation_error_out)
    }

    async fn create_group(&self) -> Result<String, ConversationError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostConversation::create_group(&mut *state).await.map_err(convert::conversation_error_out)
    }

    async fn add_member(
        &self,
        conversation: String,
        member_address: String,
    ) -> Result<(), ConversationError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostConversation::add_member(&mut *state, conversation, member_address)
            .await
            .map_err(convert::conversation_error_out)
    }

    async fn remove_member(
        &self,
        conversation: String,
        member_address: String,
    ) -> Result<(), ConversationError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostConversation::remove_member(&mut *state, conversation, member_address)
            .await
            .map_err(convert::conversation_error_out)
    }

    async fn members(&self, conversation: String) -> Result<Vec<String>, ConversationError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostConversation::members(&mut *state, conversation)
            .await
            .map_err(convert::conversation_error_out)
    }

    async fn membership_history(
        &self,
        conversation: String,
    ) -> Result<Vec<MembershipEvent>, ConversationError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostConversation::membership_history(&mut *state, conversation)
            .await
            .map(|v| v.into_iter().map(convert::membership_event_out).collect())
            .map_err(convert::conversation_error_out)
    }

    async fn sync_now(&self, conversation: String) -> Result<(), ConversationError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostConversation::sync_now(&mut *state, conversation)
            .await
            .map_err(convert::conversation_error_out)
    }
}

impl AppProxy for NativeAppHost {
    async fn call(
        &self,
        target: CallTarget,
        interface: String,
        method: String,
        params: String,
        options: Option<CallOptions>,
    ) -> Result<String, ProxyError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostProxy::call(
            &mut *state,
            convert::call_target_in(target),
            interface,
            method,
            params,
            options.map(convert::call_options_in),
        )
        .await
        .map_err(convert::proxy_error_out)
    }

    async fn enqueue(
        &self,
        target: CallTarget,
        interface: String,
        method: String,
        params: String,
        options: Option<CallOptions>,
    ) -> Result<(), ProxyError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostProxy::enqueue(
            &mut *state,
            convert::call_target_in(target),
            interface,
            method,
            params,
            options.map(convert::call_options_in),
        )
        .await
        .map_err(convert::proxy_error_out)
    }
}

impl AppAppConfig for NativeAppHost {
    async fn get(&self, key: String) -> Result<Option<String>, ConfigError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostAppConfig::get(&mut *state, key).await.map_err(convert::config_error_out)
    }

    async fn get_section(&self, prefix: String) -> Result<Vec<(String, String)>, ConfigError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostAppConfig::get_section(&mut *state, prefix).await.map_err(convert::config_error_out)
    }
}

impl AppVault for NativeAppHost {
    async fn reveal(&self, key: String) -> Result<Vec<u8>, VaultError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostVault::reveal(&mut *state, key).await.map_err(convert::vault_error_out)
    }
}

impl AppSigning for NativeAppHost {
    async fn sign_record(
        &self,
        draft: RecordDraft,
        as_principal: Principal,
    ) -> Result<String, SigningError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostSigning::sign_record(
            &mut *state,
            convert::draft_out(draft),
            convert::principal_out(as_principal),
        )
        .await
        .map_err(convert::signing_error_guest)
    }

    async fn signing_identity(&self) -> Result<SigningIdentity, SigningError> {
        let mut state = self.0.state_mutex().await.lock().await;
        HostSigning::identity(&mut *state)
            .await
            .map(convert::signing_identity_guest)
            .map_err(convert::signing_error_guest)
    }
}

impl AppWebSocket for NativeAppHost {
    async fn send(&self, conn: String, frame: Vec<u8>, kind: FrameKind) -> Result<(), String> {
        self.0.factory.websocket_senders.send(self.0.factory.service_id(), &conn, frame, kind).await
    }
}

impl AppInvocation for NativeAppHost {
    async fn caller(&self) -> CallerOrigin {
        // Mirrors `HostState`'s `invocation::Host::caller`: a local call is
        // `internal` whatever identity it carries, so the two builds
        // answer identically for a local drive. The auth level is read
        // only on the wire path -- which has no production producer on the
        // native build (see `NativeHostFactory::host_for_wire`).
        match self.0.invocation_origin {
            InvocationOrigin::Local => CallerOrigin::Internal,
            InvocationOrigin::Wire => match self.0.caller.auth {
                AuthLevel::Delegated | AuthLevel::Ucan => {
                    CallerOrigin::Verified(self.0.caller.caller_did.clone())
                }
                _ => CallerOrigin::Anonymous,
            },
        }
    }
}
