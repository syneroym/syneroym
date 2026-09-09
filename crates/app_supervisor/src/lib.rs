#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! The App Supervisor (ADR-0021 §8): a resident substrate role holding
//! desired state for the app instances it manages, exposed over the
//! `supervisor` WIT interface. It holds custody of each managed instance's
//! master key, and runs a resident loop (`service::SupervisorService::run`)
//! that reconciles those instances against their desired state. `status`
//! also sweeps on demand.

pub mod anchors;
pub mod inventory;
pub mod keys;
pub mod outbox;
pub mod service;
pub mod store;
pub mod tier1;
pub mod topology;

pub use anchors::{AnchorWriter, RegistryAnchorWriter};
pub use keys::{MasterVault, MintedMaster, VaultError};
pub use outbox::{QueueKey, SupervisorOutbox};
pub use service::SupervisorService;
pub use store::{DesiredState, SupervisorStore};
pub use tier1::{RegistryTier1Writer, Tier1Writer};
pub use topology::{ServiceTopology, TopologyBuildError, service_topology};
