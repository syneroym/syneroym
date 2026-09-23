#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! One integration suite driving the dual-build-shim fixture through both
//! builds -- the real `wasm32-wasip2` component via `AppSandboxEngine`, and
//! the same source linked in via `syneroym-app-host-native` -- and
//! asserting the results are identical. A test that passes on one build and
//! fails on the other is a bug in the shim, not in the test.

#[path = "dual_build_parity/helpers.rs"]
pub(crate) mod helpers;

#[path = "dual_build_parity/conversation.rs"]
mod conversation;
#[path = "dual_build_parity/data_layer.rs"]
mod data_layer;
#[path = "dual_build_parity/host_services.rs"]
mod host_services;
#[path = "dual_build_parity/http.rs"]
mod http;
#[path = "dual_build_parity/permitted_differences.rs"]
mod permitted_differences;
#[path = "dual_build_parity/signing.rs"]
mod signing;
