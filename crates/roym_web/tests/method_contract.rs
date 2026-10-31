#![allow(
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    unused_imports
)]

#[allow(dead_code)]
#[path = "dual_build_parity/deal_fixtures.rs"]
mod deal_fixtures;
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
#[path = "method_contract/shapes_catalog.rs"]
mod shapes_catalog;
#[path = "method_contract/shapes_directory.rs"]
mod shapes_directory;
#[path = "method_contract/shapes_transaction.rs"]
mod shapes_transaction;
