wasmtime::component::bindgen!({
    path: "wit/conversation",
    world: "conversation-host",
    additional_derives: [serde::Serialize, serde::Deserialize],
    imports: { default: async },
    exports: { default: async },
});
