//! Blob store WIT bindings
//!
//! Contains generated bindings allowing guest applications to store and
//! retrieve content-addressed blobs.

wit_bindgen::generate!({
    world: "blob-store-guest",
    path: "wit/blob-store/blob-store.wit",
    additional_derives: [serde::Serialize, serde::Deserialize]
});
