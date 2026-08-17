wasmtime::component::bindgen!({
    path: "wit/http",
    world: "http-host",
    additional_derives: [serde::Serialize, serde::Deserialize],
    imports: { default: async },
    exports: { default: async },
});
