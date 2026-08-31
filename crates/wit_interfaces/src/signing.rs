//! Guest-side bindings for `syneroym:signing`.

wit_bindgen::generate!({
    world: "signing-import",
    path: "wit/signing/signing.wit",
    additional_derives: [serde::Serialize, serde::Deserialize],
});
