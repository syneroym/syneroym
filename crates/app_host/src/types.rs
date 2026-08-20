//! The type vocabulary both builds share: the wit-bindgen *guest* types,
//! which compile for `wasm32-wasip2` and for the host alike (the host build
//! links them against stub imports it never calls).

pub mod data_layer {
    pub use syneroym_wit_interfaces::data_layer::syneroym::data_layer::store::{
        CollectionSchema, DataLayerError, IndexDefinition, IndexType, Mutation, PatchMutation,
        QueryOptions, QueryResult, RawQueryResult, RecordReadValue, RecordWriteValue, SqlValue,
    };
}

pub mod blob_store {
    pub use syneroym_wit_interfaces::blob_store::syneroym::blob_store::blob_store::BlobError;
}

pub mod messaging {
    pub use syneroym_wit_interfaces::messaging::syneroym::messaging::host_api::MessagingError;
}
