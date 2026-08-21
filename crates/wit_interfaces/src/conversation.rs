//! Guest-side bindings. `conversation-import`, not `conversation-guest`:
//! this module exists to be *called into* by other components' own worlds,
//! and `guest-api`'s export requirement would otherwise become an unmet
//! requirement of every consumer's linked component.

wit_bindgen::generate!({
    world: "conversation-import",
    path: "wit/conversation/conversation.wit",
    additional_derives: [serde::Serialize, serde::Deserialize]
});
