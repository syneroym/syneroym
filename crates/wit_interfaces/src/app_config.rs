wit_bindgen::generate!({
    path: "wit/app-config/app-config.wit",
    world: "app-config-guest",
    additional_derives: [serde::Serialize, serde::Deserialize],
});
