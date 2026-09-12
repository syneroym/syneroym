//! Configuration types for the Syneroym substrate.

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

pub mod base;
pub mod roles;
pub mod sandbox;

#[cfg(test)]
mod tests;

pub use base::*;
pub use roles::*;
pub use sandbox::*;

pub const DEFAULT_SUBSTRATE_KEY_FILE: &str = "substrate.key";
/// Implicitly discovered under `app_data_dir` when `[identity].agreement`
/// is unset -- `roymctl substrate claim`'s default output path, so claiming
/// a node and restarting it establishes ownership with no config edit.
pub const DEFAULT_CONTROLLER_AGREEMENT_FILE: &str = "agreement.json";

fn default_app_config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("syneroym")
}

fn default_app_local_data_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("syneroym")
}

fn default_app_data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("syneroym")
}

fn default_app_cache_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".")).join("syneroym")
}

fn default_app_log_dir() -> PathBuf {
    default_app_local_data_dir().join("logs")
}

const fn default_config_version() -> u32 {
    1
}

fn default_profile() -> String {
    "full".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SubstrateConfig {
    pub config_version: u32,

    pub app_config_dir: PathBuf,
    pub app_local_data_dir: PathBuf,
    pub app_data_dir: PathBuf,
    pub app_cache_dir: PathBuf,
    pub app_log_dir: PathBuf,

    pub profile: String,

    pub identity: IdentityConfig,

    pub storage: StorageConfig,
    pub logging: LoggingConfig,

    pub parent_coordinator: ParentCoordinatorConfig,

    pub profiles: HashMap<String, ProfileConfig>,

    pub roles: RolesConfig,
    pub substrate: SubstrateGlobalConfig,
    pub retry: RetryPolicy,
    pub tls: Option<SubstrateTlsConfig>,
    /// Embedded MQTT broker for `syneroym:messaging` (ADR-0010). A core,
    /// always-on capability -- not an optional deployment role like
    /// `RolesConfig`'s members.
    pub mqtt: MessagingConfig,
    /// Bidirectional stream protocols (ADR-0014). A core, always-on
    /// capability, mirroring `mqtt`'s placement above.
    pub streaming: StreamingConfig,
    /// Identity/capability admission (ADR-0015/0016).
    pub iam: IamConfig,
}

/// Useful helper functions
impl SubstrateConfig {
    /// Returns the directory where hosted app certificates are stored
    pub fn hosted_apps_dir(&self) -> PathBuf {
        self.app_local_data_dir.join("hosted_apps")
    }

    /// Resolves relative storage paths by prepending `app_local_data_dir`.
    pub fn resolve_paths(&mut self) {
        if self.storage.db_dir.is_relative() {
            self.storage.db_dir = self.app_local_data_dir.join(&self.storage.db_dir);
        }

        if self.storage.blobs_dir.is_relative() {
            self.storage.blobs_dir = self.app_local_data_dir.join(&self.storage.blobs_dir);
        }

        if self.storage.blob_store.local_root.is_relative() {
            self.storage.blob_store.local_root =
                self.app_local_data_dir.join(&self.storage.blob_store.local_root);
        }

        if let Some(key) = &self.identity.key
            && key.is_relative()
        {
            self.identity.key = Some(self.app_data_dir.join(key));
        }

        if let Some(agreement) = &self.identity.agreement
            && agreement.is_relative()
        {
            self.identity.agreement = Some(self.app_data_dir.join(agreement));
        }

        if let Some(coordinator) = &mut self.roles.coordinator
            && let Some(tls) = &mut coordinator.tls
        {
            if tls.cert_path.is_relative() {
                tls.cert_path = self.app_config_dir.join(&tls.cert_path);
            }
            if tls.key_path.is_relative() {
                tls.key_path = self.app_config_dir.join(&tls.key_path);
            }
        }

        if let Some(auth) = &mut self.roles.auth {
            if let Some(key_path) = &auth.key_path
                && key_path.is_relative()
            {
                auth.key_path = Some(self.app_data_dir.join(key_path));
            }
            if let Some(dir) = &auth.person_identities_dir
                && dir.is_relative()
            {
                auth.person_identities_dir = Some(self.app_data_dir.join(dir));
            }
        }

        if let Some(gateway) = &mut self.roles.client_gateway
            && let Some(fixed_del) = &gateway.fixed_delegation
            && fixed_del.is_relative()
        {
            gateway.fixed_delegation = Some(self.app_data_dir.join(fixed_del));
        }

        if let Some(tls) = &mut self.tls {
            if tls.cert_path.is_relative() {
                tls.cert_path = self.app_config_dir.join(&tls.cert_path);
            }
            if tls.key_path.is_relative() {
                tls.key_path = self.app_config_dir.join(&tls.key_path);
            }
        }
    }
}

impl Default for SubstrateConfig {
    fn default() -> Self {
        Self {
            config_version: default_config_version(),
            app_config_dir: default_app_config_dir(),
            app_local_data_dir: default_app_local_data_dir(),
            app_data_dir: default_app_data_dir(),
            app_cache_dir: default_app_cache_dir(),
            app_log_dir: default_app_log_dir(),
            profile: default_profile(),
            identity: Default::default(),
            storage: Default::default(),
            logging: Default::default(),
            parent_coordinator: Default::default(),
            profiles: Default::default(),
            roles: Default::default(),
            substrate: Default::default(),
            retry: Default::default(),
            tls: None,
            mqtt: Default::default(),
            streaming: Default::default(),
            iam: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstrateTlsConfig {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub reload_on_sigusr1: bool,
}
