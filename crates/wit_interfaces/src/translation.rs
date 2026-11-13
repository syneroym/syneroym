//! Guest-side bindings for `syneroym:translation`.

wit_bindgen::generate!({
    world: "translation-import",
    path: "wit/translation/translation.wit",
    additional_derives: [serde::Serialize, serde::Deserialize, PartialEq, Eq],
});
