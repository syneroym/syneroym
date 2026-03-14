use std::path::{Path, PathBuf};
use syneroym_identity::Identity;

/// Core domain types and traits for node control.
pub mod node {
    use super::*;

    #[derive(Debug, Clone)]
    pub struct Config {
        pub dir: PathBuf,
        pub detach: bool,
    }

    #[derive(Debug, Clone)]
    pub struct Status {
        pub is_online: bool,
        pub node_id: String,
        pub uptime_seconds: u64,
    }

    /// Control operations for a node.
    pub trait Control {
        fn init(dir: &Path) -> anyhow::Result<Identity>;
        fn start(config: &Config) -> anyhow::Result<()>;
        fn stop(dir: &Path) -> anyhow::Result<()>;
        fn status(dir: &Path) -> anyhow::Result<Status>;
    }
}

/// Core domain types and traits for managing SynApps.
pub mod app {
    use super::*;

    #[derive(Debug, Clone)]
    pub struct Manifest {
        pub id: String,
        pub version: String,
        pub entrypoint: PathBuf,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Status {
        Running,
        Stopped,
        Failed(String),
    }

    #[derive(Debug, Clone)]
    pub struct Info {
        pub id: String,
        pub status: Status,
    }

    /// Operations for the application lifecycle.
    pub trait Lifecycle {
        fn deploy(dir: &Path, app_id: &str, manifest_path: &Path) -> anyhow::Result<()>;
        fn remove(dir: &Path, app_id: &str) -> anyhow::Result<()>;
        fn start(dir: &Path, app_id: &str) -> anyhow::Result<()>;
        fn stop(dir: &Path, app_id: &str) -> anyhow::Result<()>;
        fn list(dir: &Path) -> anyhow::Result<Vec<Info>>;
    }
}

/// Core domain types and traits for network peers.
pub mod peer {
    use super::*;

    #[derive(Debug, Clone)]
    pub struct Info {
        pub id: String,
        pub address: Option<String>,
        pub latency_ms: Option<u32>,
    }

    /// Operations for managing network topology.
    pub trait Connectivity {
        fn list(dir: &Path) -> anyhow::Result<Vec<Info>>;
        fn connect(dir: &Path, peer_id: &str, address: Option<&str>) -> anyhow::Result<()>;
        fn disconnect(dir: &Path, peer_id: &str) -> anyhow::Result<()>;
    }
}

/// Core domain types and traits for cryptographic identities.
pub mod identity {
    use super::*;

    /// Operations for keystore interactions.
    pub trait Keystore {
        fn create(dir: &Path, name: &str) -> anyhow::Result<Identity>;
        fn list(dir: &Path) -> anyhow::Result<Vec<String>>;
        fn show(dir: &Path, name: &str) -> anyhow::Result<Option<Identity>>;
    }
}
