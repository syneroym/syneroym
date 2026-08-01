#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! The App Supervisor (ADR-0021 §8): a resident substrate role holding
//! desired state for the app instances it manages, exposed over the
//! `supervisor` WIT interface. This slice (M05A A5b) is the role, the
//! store, the interface, and master custody -- no autonomy: `status` sweeps
//! on demand, and the resident reconcile loop is a later slice.

pub mod inventory;
pub mod keys;
pub mod service;
pub mod store;

pub use keys::{MasterVault, MintedMaster, VaultError};
pub use service::SupervisorService;
pub use store::{DesiredState, SupervisorStore};
