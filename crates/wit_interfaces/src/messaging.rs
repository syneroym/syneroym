//! Messaging WIT bindings
//!
//! Contains generated bindings allowing guest applications to publish and
//! subscribe to the substrate's pub/sub broker.

wit_bindgen::generate!({
    // `messaging-import`, not `messaging-guest`: this module exists to be
    // *called into* by other components' own worlds (a pure import), and
    // `messaging-guest`'s `stream-types`/`guest-api` exports would become an
    // unmet requirement of every consumer's linked component -- same
    // reasoning as `data_layer.rs`'s `data-layer-import`, confirmed against
    // the real wasm32 component-encoding toolchain.
    world: "messaging-import",
    path: "wit/messaging/messaging.wit",
    additional_derives: [serde::Serialize, serde::Deserialize]
});
