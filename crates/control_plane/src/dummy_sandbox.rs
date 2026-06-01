//! Dummy Sandbox implementation for testing
//!
//! Provides an isolated, mock execution environment for validating WASM
//! component orchestration and workflows without invoking a full WASM runtime.

#[cfg(feature = "app_sandbox")]
pub use syneroym_app_sandbox::AppSandboxEngine;

/// A dummy implementation of the sandbox engine used when the feature is disabled.
/// This allows the rest of the codebase to use the engine unconditionally without #[cfg] spam.
#[cfg(not(feature = "app_sandbox"))]
#[derive(Debug, Clone)]
pub struct AppSandboxEngine;

#[cfg(not(feature = "app_sandbox"))]
impl AppSandboxEngine {
    pub async fn init(
        _config: &syneroym_core::config::SubstrateConfig,
        _endpoints: Vec<(String, String, syneroym_core::registry::SubstrateEndpoint)>,
    ) -> anyhow::Result<Self> {
        Ok(Self)
    }
}

#[cfg(feature = "podman_sandbox")]
pub use syneroym_podman_sandbox::ContainerEngine;

#[cfg(not(feature = "podman_sandbox"))]
#[derive(Debug, Clone)]
pub struct ContainerEngine;
