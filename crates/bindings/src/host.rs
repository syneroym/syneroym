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
