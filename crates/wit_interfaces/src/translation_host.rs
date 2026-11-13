//! Host-side bindings for `syneroym:translation`. A separate `bindgen!`
//! rather than a `host-environment` import, so a component that does not
//! import `translation` deploys exactly as before -- the same reason
//! `events_host` is separate.

wasmtime::component::bindgen!({
    path: "wit/translation",
    world: "translation-import",
    additional_derives: [serde::Serialize, serde::Deserialize, PartialEq, Eq],
    imports: { default: async },
});
