//! Host-side bindings for `syneroym:events`. A separate `bindgen!`
//! rather than a `host-environment` import, so a component that does not
//! import `events` deploys exactly as before -- the same reason
//! `invocation_host` is separate.

wasmtime::component::bindgen!({
    path: "wit/events",
    world: "events-import",
    additional_derives: [serde::Serialize, serde::Deserialize, PartialEq, Eq],
    imports: { default: async },
});
