//! Guest-side bindings for the Universal Proxy. `proxy-import`, not a
//! world of its own with `saga`: this module exists to be *called* by
//! other components' worlds, and pulling `saga` in would make its types a
//! requirement of every consumer that only wants `call`/`enqueue`.

wit_bindgen::generate!({
    world: "proxy-import",
    path: "wit/proxy/proxy.wit",
    additional_derives: [serde::Serialize, serde::Deserialize],
});
