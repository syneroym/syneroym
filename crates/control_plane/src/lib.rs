#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Control Plane service definitions and types
//!
//! Exposes APIs for deploying apps, managing running services,
//! and controlling the substrate environment.

pub mod config_utils;
pub mod dummy_sandbox;
pub mod http_routes;

mod service;
mod synsvc_native;

pub use http_routes::{HttpRoute, HttpRouteRegistry};
pub use service::ControlPlaneService;
pub use synsvc_native::SynSvcNativeService;
