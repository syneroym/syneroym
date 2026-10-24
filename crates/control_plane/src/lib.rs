#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Control Plane service definitions and types
//!
//! Exposes APIs for deploying apps, managing running services,
//! and controlling the substrate environment.

pub mod assets;
pub mod config_utils;
pub mod dummy_sandbox;
pub mod http_routes;

mod home_relay;
mod mailbox_pull;
mod service;
mod synsvc_native;

pub use home_relay::HomeRelayControl;
pub use mailbox_pull::MailboxPullControl;
pub use service::{
    AUTH_RESERVED_SERVICE_ID, ControlPlaneService, MAILBOX_RESERVED_SERVICE_ID,
    SUPERVISOR_RESERVED_SERVICE_ID,
};
pub use synsvc_native::SynSvcNativeService;
