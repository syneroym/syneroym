//! This module generates Rust types from the `control-plane.wit` interface.
//!
//! Even though the `ControlPlaneService` handles execution natively via dynamic
//! dispatch (`NativeService::dispatch`) rather than implementing the static `Guest`
//! trait, these generated types are still used for strongly-typed payload deserialization.
//!
//! # Example usage inside `NativeService::dispatch`:
//! ```ignore
//! ("orchestrator", "deploy") => {
//!     // 1. Deserialize the dynamic payload into the strongly-typed WIT struct
//!     let args: DeployArgs = serde_json::from_value(invocation.payload)?;
//!     
//!     // 2. Safely use the fields
//!     self._app_sandbox_engine.deploy_wasm(&args.service_id, args.manifest).await?;
//!     
//!     Ok(NativeResponse { payload: serde_json::json!({"status": "deployed"}) })
//! }
//! ```

wit_bindgen::generate!({
    world: "control-plane-service",
    path: "wit/control-plane.wit"
});
