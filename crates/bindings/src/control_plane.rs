wit_bindgen::generate!({
    world: "control-plane-service",
    path: "wit/control-plane.wit",
    additional_derives: [serde::Serialize, serde::Deserialize]
});
