#![cfg_attr(target_arch = "wasm32", allow(clippy::future_not_send))]
//! One trait per host interface. Two implementations: `guest` (this crate,
//! `wasm32` only) over `wit-bindgen` bindings, and `syneroym-app-host-native`
//! over the substrate's own host-capability implementations. An app is
//! written once against these traits and built both ways.
//!
//! **Every app's own `generate!` must remap `data-layer`/`blob-store`/
//! `messaging`'s import interfaces onto this crate's `guest` bindings**
//! (wit-bindgen's `with: { "...": syneroym_app_host::guest::wit_bindings... }`
//! -- see `guest.rs`'s own doc comment) rather than regenerating them. A
//! *second* `generate!` pass over the same import interfaces, linked into
//! the same wasm32 component, fails to encode: confirmed against the real
//! toolchain, not by inspection -- `wasm-component-ld` cannot reconcile two
//! independent copies of the same imports into one component-type section.

use core::future::Future;

#[cfg(target_arch = "wasm32")]
pub mod guest;
pub mod types;

use types::{
    blob_store::BlobError,
    conversation::{ConversationError, ConversationSummary, DeliveryState, HistoryPage, Message},
    data_layer::*,
    messaging::MessagingError,
};

/// Everything an app may reach. One bound for an app to be generic over.
///
/// Requires `AppConversation`: both implementors (`GuestHost`,
/// `NativeAppHost`) implement it for full capability access.
pub trait AppHost:
    AppDataLayer + AppBlobStore + AppMessaging + AppConversation + Send + Sync
{
}
impl<T> AppHost for T where
    T: AppDataLayer + AppBlobStore + AppMessaging + AppConversation + Send + Sync
{
}

/// Mirrors `syneroym:data-layer/store@0.1.0`, function for function.
pub trait AppDataLayer {
    fn create_collection(
        &self,
        schema: CollectionSchema,
    ) -> impl Future<Output = Result<(), DataLayerError>> + Send;

    fn drop_collection(
        &self,
        name: String,
    ) -> impl Future<Output = Result<(), DataLayerError>> + Send;

    fn put(
        &self,
        collection: String,
        value: RecordWriteValue,
    ) -> impl Future<Output = Result<(), DataLayerError>> + Send;

    fn patch(
        &self,
        collection: String,
        id: String,
        patch_json: Vec<u8>,
    ) -> impl Future<Output = Result<(), DataLayerError>> + Send;

    fn get(
        &self,
        collection: String,
        id: String,
    ) -> impl Future<Output = Result<Option<RecordReadValue>, DataLayerError>> + Send;

    fn query(
        &self,
        collection: String,
        opts: QueryOptions,
    ) -> impl Future<Output = Result<QueryResult, DataLayerError>> + Send;

    fn aggregate(
        &self,
        collection: String,
        pipeline: String,
    ) -> impl Future<Output = Result<RawQueryResult, DataLayerError>> + Send;

    fn delete(
        &self,
        collection: String,
        id: String,
    ) -> impl Future<Output = Result<(), DataLayerError>> + Send;

    fn delete_many(
        &self,
        collection: String,
        filter: String,
    ) -> impl Future<Output = Result<u64, DataLayerError>> + Send;

    fn batch_mutate(
        &self,
        collection: String,
        mutations: Vec<Mutation>,
    ) -> impl Future<Output = Result<(), DataLayerError>> + Send;

    fn execute_ddl(&self, sql: String) -> impl Future<Output = Result<(), DataLayerError>> + Send;

    fn query_raw(
        &self,
        sql: String,
        params: Vec<SqlValue>,
    ) -> impl Future<Output = Result<RawQueryResult, DataLayerError>> + Send;

    fn check_access(
        &self,
        collection: String,
        id: String,
        operation: String,
    ) -> impl Future<Output = Result<bool, DataLayerError>> + Send;
}

/// Mirrors `syneroym:blob-store/blob-store@0.1.0`. The two resources become
/// associated types: a guest holds a wit-bindgen handle, the native shim
/// holds a `ResourceTable` index plus a handle back to its host.
pub trait AppBlobStore {
    type Writer: AppBlobWriter;
    type Reader: AppBlobReader;

    fn put_blob(&self, data: Vec<u8>) -> impl Future<Output = Result<String, BlobError>> + Send;

    fn get_blob(&self, hash: String) -> impl Future<Output = Result<Vec<u8>, BlobError>> + Send;

    fn open_upload(&self) -> impl Future<Output = Result<Self::Writer, BlobError>> + Send;

    fn open_download(
        &self,
        hash: String,
        offset: u64,
    ) -> impl Future<Output = Result<Self::Reader, BlobError>> + Send;

    fn delete_blob(&self, hash: String) -> impl Future<Output = Result<(), BlobError>> + Send;

    fn signed_url(
        &self,
        hash: String,
        ttl_secs: u32,
    ) -> impl Future<Output = Result<String, BlobError>> + Send;
}

pub trait AppBlobWriter: Send {
    fn write(&mut self, chunk: Vec<u8>) -> impl Future<Output = Result<(), BlobError>> + Send;
    /// Consumes the writer: a finished upload cannot be written to again on
    /// either build (the host deletes its table entry; the guest's handle is
    /// dropped here rather than left dangling).
    fn finish(self) -> impl Future<Output = Result<String, BlobError>> + Send;
    fn abort(self) -> impl Future<Output = ()> + Send;
}

pub trait AppBlobReader: Send {
    fn read(&mut self, max_bytes: u32) -> impl Future<Output = Result<Vec<u8>, BlobError>> + Send;
}

/// Mirrors `syneroym:messaging/host-api@0.1.0`, minus
/// `register-stream-protocol`, whose only implementation registers a WASM
/// endpoint and so has no native counterpart.
pub trait AppMessaging {
    fn publish(
        &self,
        topic: String,
        payload: Vec<u8>,
    ) -> impl Future<Output = Result<(), MessagingError>> + Send;
    fn subscribe(&self, topic: String) -> impl Future<Output = Result<(), MessagingError>> + Send;
    fn unsubscribe(&self, topic: String)
    -> impl Future<Output = Result<(), MessagingError>> + Send;
}

/// The host -> app direction. The WASM build's equivalent is the exported
/// `syneroym:messaging/guest-api@0.1.0#handle-message`.
///
/// Defined here, not in the native shim crate: it is part of the app-facing
/// contract (it is how an app *receives*), and the fixture must not depend
/// on the shim crate. `syneroym-app-host-native` re-exports it. `async_trait`
/// rather than an `impl Future` return because this one is used as
/// `dyn MessageSink`.
#[async_trait::async_trait]
pub trait MessageSink: Send + Sync + core::fmt::Debug {
    async fn handle_message(&self, topic: String, payload: Vec<u8>) -> Result<(), String>;
}

/// Mirrors `syneroym:conversation/conversation@0.1.0`, function for
/// function.
pub trait AppConversation {
    fn open_direct(
        &self,
        peer_address: String,
    ) -> impl Future<Output = Result<String, ConversationError>> + Send;
    fn conversations(
        &self,
    ) -> impl Future<Output = Result<Vec<ConversationSummary>, ConversationError>> + Send;
    fn send(
        &self,
        conversation: String,
        content_type: String,
        body: Vec<u8>,
    ) -> impl Future<Output = Result<String, ConversationError>> + Send;
    fn history(
        &self,
        conversation: String,
        limit: u32,
        cursor: Option<String>,
    ) -> impl Future<Output = Result<HistoryPage, ConversationError>> + Send;
    fn delivery_status(
        &self,
        message: String,
    ) -> impl Future<Output = Result<DeliveryState, ConversationError>> + Send;
    fn outbox(&self) -> impl Future<Output = Result<Vec<Message>, ConversationError>> + Send;
    fn retry(&self, message: String) -> impl Future<Output = Result<(), ConversationError>> + Send;
}

/// The host -> app direction for conversations.
/// `MessageSink`'s shape, for the same reason: part of the app-facing
/// contract, and used as `dyn`.
#[async_trait::async_trait]
pub trait ConversationSink: Send + Sync + core::fmt::Debug {
    async fn on_message(&self, msg: Message) -> Result<(), String>;
    async fn on_delivery_state(&self, message: String, state: DeliveryState) -> Result<(), String>;
}
