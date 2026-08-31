//! Host-side bindings for `syneroym:signing`. A separate `bindgen!` rather
//! than a `host-environment` import, so a component that does not import
//! `signing` deploys exactly as before -- the same reason
//! `conversation_host` is separate.

wasmtime::component::bindgen!({
    path: "wit/signing",
    world: "signing-import",
    additional_derives: [serde::Serialize, serde::Deserialize],
    imports: { default: async },
});
