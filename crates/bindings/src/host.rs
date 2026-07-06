//! WIT host bindings
//!
//! Provides the WIT interfaces enabling the substrate host to communicate
//! with guest WASM modules.

wasmtime::component::bindgen!({
    path: "wit/host",
    world: "host-environment",
    additional_derives: [serde::Serialize, serde::Deserialize],
    imports: {
        default: async,
    },
    exports: {
        default: async,
    }
});
