//! Messaging WIT bindings
//!
//! Contains generated bindings allowing guest applications to publish and
//! subscribe to the substrate's pub/sub broker.

wit_bindgen::generate!({
    world: "messaging-guest",
    path: "wit/messaging/messaging.wit",
    additional_derives: [serde::Serialize, serde::Deserialize]
});
