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
        _config: &SubstrateConfig,
        _endpoints: Vec<(String, SubstrateEndpoint)>,
    ) -> Result<Self> {
        Ok(Self)
    }

    // Add any dummy methods here as you build out the real AppSandboxEngine
    // pub async fn deploy_wasm(&self, _channel_id: &str) -> Result<()> { Ok(()) }
}
