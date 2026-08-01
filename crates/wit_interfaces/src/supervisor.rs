//! Supervisor WIT bindings
//!
//! Contains generated bindings for the App Supervisor's own interface,
//! consumed by `roymctl` and dispatched by `syneroym-app-supervisor`.

wit_bindgen::generate!({
    world: "supervisor-service",
    path: "wit/supervisor/supervisor.wit",
    additional_derives: [serde::Serialize, serde::Deserialize]
});
