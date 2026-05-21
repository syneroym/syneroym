//! wRPC host bindings
//!
//! Provides the wRPC interfaces enabling the substrate host to communicate
//! with guest WASM modules.

wasmtime::component::bindgen!({
    path: "wit/host.wit",
    world: "host-environment",
    imports: {
        default: async,
    },
    exports: {
        default: async,
    }
});
