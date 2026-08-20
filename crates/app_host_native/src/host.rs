//! [`NativeAppHost`]: one invocation's handle to the host, and the trait
//! impls that delegate every call to `HostState`'s existing `Host` impls --
//! the same semantics the WASM build reaches, through the same code.

use std::{fmt, sync::Arc};

use syneroym_app_host::{
    AppBlobReader, AppBlobStore, AppBlobWriter, AppDataLayer, AppMessaging,
    types::{blob_store::BlobError, data_layer::*, messaging::MessagingError},
};
use syneroym_sandbox_wasm::HostState;
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
/// `Debug` impl and `tokio::sync::Mutex<T>: Debug where T: Debug` (`HostState`
/// supplies its own).
#[derive(Debug)]
pub(crate) struct HostInner {
    pub(crate) factory: Arc<NativeHostFactory>,
    /// `tokio::sync::Mutex`, not `std`: the guarded calls are async, and the
    /// guard is held across them. `HostState` is `Send` (wasmtime requires it
    /// for async stores) but not `Sync`, which is exactly what a `Mutex`
    /// fixes.
    pub(crate) state: tokio::sync::Mutex<HostState>,
}

impl AppDataLayer for NativeAppHost {
    async fn create_collection(&self, schema: CollectionSchema) -> Result<(), DataLayerError> {
        use syneroym_wit_interfaces::host::syneroym::data_layer::store::Host as HostStore;
        let mut state = self.0.state.lock().await;
        HostStore::create_collection(&mut *state, convert::collection_schema_in(schema))
            .await
            .map_err(convert::data_layer_error_out)
    }

    async fn drop_collection(&self, name: String) -> Result<(), DataLayerError> {
        use syneroym_wit_interfaces::host::syneroym::data_layer::store::Host as HostStore;
        let mut state = self.0.state.lock().await;
        HostStore::drop_collection(&mut *state, name).await.map_err(convert::data_layer_error_out)
    }

    async fn put(&self, collection: String, value: RecordWriteValue) -> Result<(), DataLayerError> {
        use syneroym_wit_interfaces::host::syneroym::data_layer::store::Host as HostStore;
        let mut state = self.0.state.lock().await;
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
        use syneroym_wit_interfaces::host::syneroym::data_layer::store::Host as HostStore;
        let mut state = self.0.state.lock().await;
        HostStore::patch(&mut *state, collection, id, patch_json)
            .await
            .map_err(convert::data_layer_error_out)
    }

    async fn get(
        &self,
        collection: String,
        id: String,
    ) -> Result<Option<RecordReadValue>, DataLayerError> {
        use syneroym_wit_interfaces::host::syneroym::data_layer::store::Host as HostStore;
        let mut state = self.0.state.lock().await;
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
        use syneroym_wit_interfaces::host::syneroym::data_layer::store::Host as HostStore;
        let mut state = self.0.state.lock().await;
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
        use syneroym_wit_interfaces::host::syneroym::data_layer::store::Host as HostStore;
        let mut state = self.0.state.lock().await;
        HostStore::aggregate(&mut *state, collection, pipeline)
            .await
            .map(convert::raw_query_result_out)
            .map_err(convert::data_layer_error_out)
    }

    async fn delete(&self, collection: String, id: String) -> Result<(), DataLayerError> {
        use syneroym_wit_interfaces::host::syneroym::data_layer::store::Host as HostStore;
        let mut state = self.0.state.lock().await;
        HostStore::delete(&mut *state, collection, id).await.map_err(convert::data_layer_error_out)
    }

    async fn delete_many(&self, collection: String, filter: String) -> Result<u64, DataLayerError> {
        use syneroym_wit_interfaces::host::syneroym::data_layer::store::Host as HostStore;
        let mut state = self.0.state.lock().await;
        HostStore::delete_many(&mut *state, collection, filter)
            .await
            .map_err(convert::data_layer_error_out)
    }

    async fn batch_mutate(
        &self,
        collection: String,
        mutations: Vec<Mutation>,
    ) -> Result<(), DataLayerError> {
        use syneroym_wit_interfaces::host::syneroym::data_layer::store::Host as HostStore;
        let mut state = self.0.state.lock().await;
        HostStore::batch_mutate(
            &mut *state,
            collection,
            mutations.into_iter().map(convert::mutation_in).collect(),
        )
        .await
        .map_err(convert::data_layer_error_out)
    }

    async fn execute_ddl(&self, sql: String) -> Result<(), DataLayerError> {
        use syneroym_wit_interfaces::host::syneroym::data_layer::store::Host as HostStore;
        let mut state = self.0.state.lock().await;
        HostStore::execute_ddl(&mut *state, sql).await.map_err(convert::data_layer_error_out)
    }

    async fn query_raw(
        &self,
        sql: String,
        params: Vec<SqlValue>,
    ) -> Result<RawQueryResult, DataLayerError> {
        use syneroym_wit_interfaces::host::syneroym::data_layer::store::Host as HostStore;
        let mut state = self.0.state.lock().await;
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
        use syneroym_wit_interfaces::host::syneroym::data_layer::store::Host as HostStore;
        let mut state = self.0.state.lock().await;
        HostStore::check_access(&mut *state, collection, id, operation)
            .await
            .map_err(convert::data_layer_error_out)
    }
}

impl AppBlobStore for NativeAppHost {
    type Writer = NativeBlobWriter;
    type Reader = NativeBlobReader;

    async fn put_blob(&self, data: Vec<u8>) -> Result<String, BlobError> {
        use syneroym_wit_interfaces::host::syneroym::blob_store::blob_store::Host as HostBlobStore;
        let mut state = self.0.state.lock().await;
        HostBlobStore::put_blob(&mut *state, data).await.map_err(convert::blob_error_out)
    }

    async fn get_blob(&self, hash: String) -> Result<Vec<u8>, BlobError> {
        use syneroym_wit_interfaces::host::syneroym::blob_store::blob_store::Host as HostBlobStore;
        let mut state = self.0.state.lock().await;
        HostBlobStore::get_blob(&mut *state, hash).await.map_err(convert::blob_error_out)
    }

    async fn open_upload(&self) -> Result<NativeBlobWriter, BlobError> {
        use syneroym_wit_interfaces::host::syneroym::blob_store::blob_store::Host as HostBlobStore;
        let rep = {
            let mut state = self.0.state.lock().await;
            HostBlobStore::open_upload(&mut *state).await.map_err(convert::blob_error_out)?.rep()
        };
        Ok(NativeBlobWriter { host: self.clone(), rep })
    }

    async fn open_download(
        &self,
        hash: String,
        offset: u64,
    ) -> Result<NativeBlobReader, BlobError> {
        use syneroym_wit_interfaces::host::syneroym::blob_store::blob_store::Host as HostBlobStore;
        let rep = {
            let mut state = self.0.state.lock().await;
            HostBlobStore::open_download(&mut *state, hash, offset)
                .await
                .map_err(convert::blob_error_out)?
                .rep()
        };
        Ok(NativeBlobReader { host: self.clone(), rep })
    }

    async fn delete_blob(&self, hash: String) -> Result<(), BlobError> {
        use syneroym_wit_interfaces::host::syneroym::blob_store::blob_store::Host as HostBlobStore;
        let mut state = self.0.state.lock().await;
        HostBlobStore::delete_blob(&mut *state, hash).await.map_err(convert::blob_error_out)
    }

    async fn signed_url(&self, hash: String, ttl_secs: u32) -> Result<String, BlobError> {
        use syneroym_wit_interfaces::host::syneroym::blob_store::blob_store::Host as HostBlobStore;
        let mut state = self.0.state.lock().await;
        HostBlobStore::signed_url(&mut *state, hash, ttl_secs)
            .await
            .map_err(convert::blob_error_out)
    }
}

/// The `rep` (rather than a stored `Resource<T>`) is deliberate: the host
/// methods take the resource handle by value, and rebuilding
/// `Resource::new_own(rep)` per call is what `HostBlobWriter`'s own
/// implementation expects -- `write` does `table.get_mut(&self_)` and
/// `finish`/`abort` do `table.delete(self_)`, so the table, not the handle,
/// owns lifetime; both only ever read the raw index back out via `rep()`.
pub struct NativeBlobWriter {
    host: NativeAppHost,
    rep: u32,
}

impl fmt::Debug for NativeBlobWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeBlobWriter").field("rep", &self.rep).finish_non_exhaustive()
    }
}

impl AppBlobWriter for NativeBlobWriter {
    async fn write(&mut self, chunk: Vec<u8>) -> Result<(), BlobError> {
        use syneroym_wit_interfaces::host::syneroym::blob_store::blob_store::HostBlobWriter;
        let mut state = self.host.0.state.lock().await;
        HostBlobWriter::write(&mut *state, Resource::new_own(self.rep), chunk)
            .await
            .map_err(convert::blob_error_out)
    }

    async fn finish(self) -> Result<String, BlobError> {
        use syneroym_wit_interfaces::host::syneroym::blob_store::blob_store::HostBlobWriter;
        let mut state = self.host.0.state.lock().await;
        HostBlobWriter::finish(&mut *state, Resource::new_own(self.rep))
            .await
            .map_err(convert::blob_error_out)
    }

    async fn abort(self) {
        use syneroym_wit_interfaces::host::syneroym::blob_store::blob_store::HostBlobWriter;
        let mut state = self.host.0.state.lock().await;
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
        use syneroym_wit_interfaces::host::syneroym::blob_store::blob_store::HostBlobReader;
        let mut state = self.host.0.state.lock().await;
        HostBlobReader::read(&mut *state, Resource::new_own(self.rep), max_bytes)
            .await
            .map_err(convert::blob_error_out)
    }
}

impl AppMessaging for NativeAppHost {
    async fn publish(&self, topic: String, payload: Vec<u8>) -> Result<(), MessagingError> {
        use syneroym_wit_interfaces::host::syneroym::messaging::host_api::Host as HostMessaging;
        let mut state = self.0.state.lock().await;
        HostMessaging::publish(&mut *state, topic, payload).await.map_err(convert::msg_error_out)
    }

    async fn subscribe(&self, topic: String) -> Result<(), MessagingError> {
        // The factory implements `subscribe`'s semantics itself (it never
        // routes through `HostState`, see its own doc comment), so it
        // already speaks the guest-vocabulary `MessagingError` -- no
        // `convert::msg_error_out` needed here, unlike `publish` above.
        self.0.factory.subscribe(topic).await
    }

    async fn unsubscribe(&self, topic: String) -> Result<(), MessagingError> {
        self.0.factory.unsubscribe(&topic)
    }
}
