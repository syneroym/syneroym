#![cfg(target_arch = "wasm32")]
//! The `wit-bindgen` implementation of the traits. Every call here is
//! synchronous at the ABI, so every future returned is already complete --
//! see `block_on`.
//!
//! Calls `syneroym-wit-interfaces`'s pre-generated import bindings directly
//! rather than regenerating them here, and every app's own `generate!` must
//! do the same (remap these three interfaces onto this crate's `guest`
//! bindings via wit-bindgen's `with:` option, `generate` only its own
//! export-side interfaces) -- a *second* `generate!` pass over the same
//! import interfaces, linked into the same component, fails to encode
//! (confirmed against the real toolchain: `wasm-component-ld` cannot
//! reconcile two independent copies of the same imports into one
//! component-type section).

use core::fmt;

use syneroym_wit_interfaces::{
    blob_store::syneroym::blob_store::blob_store as bs,
    conversation::syneroym::conversation::conversation as conv,
    data_layer::syneroym::data_layer::store as dl, messaging::syneroym::messaging::host_api as msg,
};

use crate::{
    AppBlobReader, AppBlobStore, AppBlobWriter, AppConversation, AppDataLayer, AppMessaging,
    types::{
        blob_store::BlobError, conversation::ConversationError, data_layer::*,
        messaging::MessagingError,
    },
};

/// The app's handle to the host in the WASM build. Zero-sized: the component
/// model already binds the imports.
#[derive(Debug, Clone, Copy, Default)]
pub struct GuestHost;

impl AppDataLayer for GuestHost {
    async fn create_collection(&self, schema: CollectionSchema) -> Result<(), DataLayerError> {
        dl::create_collection(&schema)
    }

    async fn drop_collection(&self, name: String) -> Result<(), DataLayerError> {
        dl::drop_collection(&name)
    }

    async fn put(&self, collection: String, value: RecordWriteValue) -> Result<(), DataLayerError> {
        dl::put(&collection, &value)
    }

    async fn patch(
        &self,
        collection: String,
        id: String,
        patch_json: Vec<u8>,
    ) -> Result<(), DataLayerError> {
        dl::patch(&collection, &id, &patch_json)
    }

    async fn get(
        &self,
        collection: String,
        id: String,
    ) -> Result<Option<RecordReadValue>, DataLayerError> {
        dl::get(&collection, &id)
    }

    async fn query(
        &self,
        collection: String,
        opts: QueryOptions,
    ) -> Result<QueryResult, DataLayerError> {
        dl::query(&collection, &opts)
    }

    async fn aggregate(
        &self,
        collection: String,
        pipeline: String,
    ) -> Result<RawQueryResult, DataLayerError> {
        dl::aggregate(&collection, &pipeline)
    }

    async fn delete(&self, collection: String, id: String) -> Result<(), DataLayerError> {
        dl::delete(&collection, &id)
    }

    async fn delete_many(&self, collection: String, filter: String) -> Result<u64, DataLayerError> {
        dl::delete_many(&collection, &filter)
    }

    async fn batch_mutate(
        &self,
        collection: String,
        mutations: Vec<Mutation>,
    ) -> Result<(), DataLayerError> {
        dl::batch_mutate(&collection, &mutations)
    }

    async fn execute_ddl(&self, sql: String) -> Result<(), DataLayerError> {
        dl::execute_ddl(&sql)
    }

    async fn query_raw(
        &self,
        sql: String,
        params: Vec<SqlValue>,
    ) -> Result<RawQueryResult, DataLayerError> {
        dl::query_raw(&sql, &params)
    }

    async fn check_access(
        &self,
        collection: String,
        id: String,
        operation: String,
    ) -> Result<bool, DataLayerError> {
        dl::check_access(&collection, &id, &operation)
    }
}

impl AppBlobStore for GuestHost {
    type Writer = GuestBlobWriter;
    type Reader = GuestBlobReader;

    async fn put_blob(&self, data: Vec<u8>) -> Result<String, BlobError> {
        bs::put_blob(&data)
    }

    async fn get_blob(&self, hash: String) -> Result<Vec<u8>, BlobError> {
        bs::get_blob(&hash)
    }

    async fn open_upload(&self) -> Result<GuestBlobWriter, BlobError> {
        bs::open_upload().map(GuestBlobWriter)
    }

    async fn open_download(&self, hash: String, offset: u64) -> Result<GuestBlobReader, BlobError> {
        bs::open_download(&hash, offset).map(GuestBlobReader)
    }

    async fn delete_blob(&self, hash: String) -> Result<(), BlobError> {
        bs::delete_blob(&hash)
    }

    async fn signed_url(&self, hash: String, ttl_secs: u32) -> Result<String, BlobError> {
        bs::signed_url(&hash, ttl_secs)
    }
}

pub struct GuestBlobWriter(bs::BlobWriter);

/// Hand-written: the generated `bs::BlobWriter` resource handle has no
/// `Debug` impl of its own.
impl fmt::Debug for GuestBlobWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuestBlobWriter").finish_non_exhaustive()
    }
}

impl AppBlobWriter for GuestBlobWriter {
    async fn write(&mut self, chunk: Vec<u8>) -> Result<(), BlobError> {
        self.0.write(&chunk)
    }

    async fn finish(self) -> Result<String, BlobError> {
        self.0.finish() // `self` dropped here: no second call possible
    }

    async fn abort(self) {
        self.0.abort();
    }
}

pub struct GuestBlobReader(bs::BlobReader);

/// Hand-written: the generated `bs::BlobReader` resource handle has no
/// `Debug` impl of its own.
impl fmt::Debug for GuestBlobReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuestBlobReader").finish_non_exhaustive()
    }
}

impl AppBlobReader for GuestBlobReader {
    async fn read(&mut self, max_bytes: u32) -> Result<Vec<u8>, BlobError> {
        self.0.read(max_bytes)
    }
}

impl AppMessaging for GuestHost {
    async fn publish(&self, topic: String, payload: Vec<u8>) -> Result<(), MessagingError> {
        msg::publish(&topic, &payload)
    }

    async fn subscribe(&self, topic: String) -> Result<(), MessagingError> {
        msg::subscribe(&topic)
    }

    async fn unsubscribe(&self, topic: String) -> Result<(), MessagingError> {
        msg::unsubscribe(&topic)
    }
}

impl AppConversation for GuestHost {
    async fn open_direct(&self, peer_address: String) -> Result<String, ConversationError> {
        conv::open_direct(&peer_address)
    }

    async fn conversations(
        &self,
    ) -> Result<Vec<crate::types::conversation::ConversationSummary>, ConversationError> {
        conv::conversations()
    }

    async fn send(
        &self,
        conversation: String,
        content_type: String,
        body: Vec<u8>,
    ) -> Result<String, ConversationError> {
        conv::send(&conversation, &content_type, &body)
    }

    async fn history(
        &self,
        conversation: String,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<crate::types::conversation::HistoryPage, ConversationError> {
        conv::history(&conversation, limit, cursor.as_deref())
    }

    async fn delivery_status(
        &self,
        message: String,
    ) -> Result<crate::types::conversation::DeliveryState, ConversationError> {
        conv::delivery_status(&message)
    }

    async fn outbox(&self) -> Result<Vec<crate::types::conversation::Message>, ConversationError> {
        conv::outbox()
    }

    async fn retry(&self, message: String) -> Result<(), ConversationError> {
        conv::retry(&message)
    }

    async fn create_group(&self) -> Result<String, ConversationError> {
        conv::create_group()
    }

    async fn add_member(
        &self,
        conversation: String,
        member_address: String,
    ) -> Result<(), ConversationError> {
        conv::add_member(&conversation, &member_address)
    }

    async fn remove_member(
        &self,
        conversation: String,
        member_address: String,
    ) -> Result<(), ConversationError> {
        conv::remove_member(&conversation, &member_address)
    }

    async fn members(&self, conversation: String) -> Result<Vec<String>, ConversationError> {
        conv::members(&conversation)
    }

    async fn membership_history(
        &self,
        conversation: String,
    ) -> Result<Vec<crate::types::conversation::MembershipEvent>, ConversationError> {
        conv::membership_history(&conversation)
    }

    async fn sync_now(&self, conversation: String) -> Result<(), ConversationError> {
        conv::sync_now(&conversation)
    }
}

/// Drives an already-complete future to its value.
///
/// Correct only because every future this crate's guest implementations
/// return is complete on first poll: each wraps one synchronous component-
/// model call. A `Pending` here means an app awaited something that is not a
/// host call, which the WASM build cannot support -- so it panics loudly
/// rather than spinning.
pub fn block_on<F: core::future::Future>(fut: F) -> F::Output {
    use core::{pin::pin, task::Poll};
    let waker = core::task::Waker::noop();
    match pin!(fut).poll(&mut core::task::Context::from_waker(waker)) {
        Poll::Ready(v) => v,
        // Asserts an invariant this crate's own guest impls guarantee
        // (every returned future is complete on first poll), not a
        // reachable runtime error -- see the doc comment above.
        #[allow(clippy::panic)]
        Poll::Pending => panic!(
            "guest future pended: the WASM build can only await host calls, which never pend"
        ),
    }
}
