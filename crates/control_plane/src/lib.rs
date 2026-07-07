#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Control Plane service definitions and types
//!
//! Exposes APIs for deploying apps, managing running services,
//! and controlling the substrate environment.

pub mod config_utils;
pub mod dummy_sandbox;

mod service;

pub use service::ControlPlaneService;
