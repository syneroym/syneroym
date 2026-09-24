//! Shared test fixtures and helpers for `crates/sandbox_wasm/tests/`
//! integration tests.

#![allow(dead_code)]

use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};

/// Builds a [`DeployManifest`] for a WASM component with the specified byte
/// payload and exposed interfaces.
pub fn wasm_deploy_manifest(bytes: Vec<u8>, interfaces: Vec<String>) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(bytes),
            hash: None,
            interfaces,
        }),
        registry_certificate: None,
        instance_certificate: None,
    }
}
