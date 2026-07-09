//! Data layer WIT bindings
//!
//! Contains generated bindings and marshalling traits allowing guest
//! applications to invoke substrate data layer features.

wit_bindgen::generate!({
    world: "data-layer-guest",
    path: "wit/data-layer/data-layer.wit",
    additional_derives: [serde::Serialize, serde::Deserialize]
});
