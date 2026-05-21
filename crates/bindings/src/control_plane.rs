//! wRPC control plane bindings
//!
//! Contains generated bindings and marshalling traits allowing guest applications
//! to invoke substrate control plane features.

wit_bindgen::generate!({
    world: "control-plane-service",
    path: "wit/control-plane.wit",
    additional_derives: [serde::Serialize, serde::Deserialize]
});
