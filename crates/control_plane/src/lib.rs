//! Control Plane service definitions and types
//!
//! Exposes APIs for deploying apps, managing running services,
//! and controlling the substrate environment.

pub mod dummy_sandbox;
mod service;

pub use service::ControlPlaneService;
