//! Guest-side bindings for `syneroym:events`.

wit_bindgen::generate!({
    world: "events-import",
    path: "wit/events/events.wit",
    additional_derives: [serde::Serialize, serde::Deserialize, PartialEq, Eq],
});
