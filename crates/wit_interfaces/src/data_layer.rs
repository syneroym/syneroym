//! Data layer WIT bindings
//!
//! Contains generated bindings and marshalling traits allowing guest
//! applications to invoke substrate data layer features.

wit_bindgen::generate!({
    // `data-layer-import`, not `data-layer-guest`: this module exists to be
    // *called into* by other components' own worlds (a pure import), and
    // `data-layer-guest`'s `init`/`migrate` exports would become an unmet
    // requirement of every consumer's linked component -- confirmed against
    // the real wasm32 component-encoding toolchain, not by inspection.
    world: "data-layer-import",
    path: "wit/data-layer/data-layer.wit",
    additional_derives: [serde::Serialize, serde::Deserialize]
});
