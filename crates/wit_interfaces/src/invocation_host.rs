//! Host-side bindings for `syneroym:invocation`. A separate `bindgen!`
//! rather than a `host-environment` import, so a component that does not
//! import `invocation` deploys exactly as before -- the same reason
//! `signing_host` is separate.

wasmtime::component::bindgen!({
    path: "wit/invocation",
    world: "invocation-import",
    additional_derives: [serde::Serialize, serde::Deserialize, PartialEq, Eq],
    imports: { default: async },
});
