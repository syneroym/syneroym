//! Application sandbox engine for isolating user applications.

use syneroym_core::config::SubstrateConfig;

pub struct AppSandboxEngine {}

impl AppSandboxEngine {
    pub fn new(_config: &SubstrateConfig) -> Self {
        Self {}
    }
}
