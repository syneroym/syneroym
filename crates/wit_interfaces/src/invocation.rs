//! Guest-side bindings for `syneroym:invocation`.

wit_bindgen::generate!({
    world: "invocation-import",
    path: "wit/invocation/invocation.wit",
    additional_derives: [serde::Serialize, serde::Deserialize, PartialEq, Eq],
});
