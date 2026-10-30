#![allow(
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    unused_imports
)]

#[allow(dead_code)]
#[path = "dual_build_parity/fixtures.rs"]
mod fixtures;
#[allow(dead_code)]
#[path = "dual_build_parity/helpers.rs"]
mod helpers;
#[allow(dead_code)]
#[path = "dual_build_parity/trust_fixtures.rs"]
mod trust_fixtures;
#[allow(dead_code)]
#[path = "dual_build_parity/trust_harness.rs"]
mod trust_harness;

#[path = "method_contract/contract.rs"]
mod contract;
#[path = "method_contract/shapes.rs"]
mod shapes;
