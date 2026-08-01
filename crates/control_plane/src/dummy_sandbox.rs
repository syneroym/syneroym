//! Dummy Sandbox implementation for testing

//! Provides an isolated, mock execution environment for validating WASM
//! component orchestration and workflows without invoking a full WASM runtime.

#[cfg(not(feature = "app_sandbox"))]
use syneroym_core::config::SubstrateConfig;
#[cfg(not(feature = "app_sandbox"))]
use syneroym_core::local_registry::SubstrateEndpoint;
#[cfg(feature = "app_sandbox")]
pub use syneroym_sandbox_wasm::AppSandboxEngine;

/// A dummy implementation of the sandbox engine used when the feature is
/// disabled. This allows the rest of the codebase to use the engine
/// unconditionally without #[cfg] spam.
#[cfg(not(feature = "app_sandbox"))]
#[derive(Debug, Clone)]
pub struct AppSandboxEngine;

#[cfg(not(feature = "app_sandbox"))]
impl AppSandboxEngine {
    pub async fn init(
        _config: &SubstrateConfig,
        _endpoints: Vec<(String, String, SubstrateEndpoint)>,
    ) -> anyhow::Result<Self> {
        Ok(Self)
    }

    /// Mirrors `syneroym_sandbox_wasm::AppSandboxEngine::exports_authorize_rows`:
    /// a sandbox-less build never has a compiled component to inspect, so a
    /// policy that opts a permission into the stage-4 after-step
    /// (`authorize_rows: true`, ADR-0017 §7) always fails
    /// `validate_stage4_export`'s deploy-time gate here, rather than
    /// silently compiling and denying every read at runtime.
    #[must_use]
    pub fn exports_authorize_rows(&self, _service_id: &str) -> bool {
        false
    }

    /// A sandbox-less build never has a loaded component, mirroring
    /// `exports_authorize_rows`'s own reasoning above (M05A A4).
    #[must_use]
    pub fn is_deployed(&self, _service_id: &str) -> bool {
        false
    }
}

#[cfg(feature = "podman_sandbox")]
pub use syneroym_sandbox_podman::ContainerEngine;

#[cfg(not(feature = "podman_sandbox"))]
#[derive(Debug, Clone)]
pub struct ContainerEngine;

#[cfg(not(feature = "podman_sandbox"))]
impl ContainerEngine {
    /// A build with no container engine cannot answer a readiness question
    /// about a container it could never have started (M05A A4).
    pub async fn readyz(&self, service_id: &str) -> anyhow::Result<()> {
        anyhow::bail!("container support is not compiled into this substrate ({service_id})")
    }
}
