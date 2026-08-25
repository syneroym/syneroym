//! Guest-side bindings for WebSocket send import.

wit_bindgen::generate!({
    world: "websocket-import",
    path: "wit/http",
    additional_derives: [serde::Serialize, serde::Deserialize],
});
