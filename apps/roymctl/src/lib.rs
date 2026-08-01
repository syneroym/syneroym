#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Library surface for `roymctl` (M05A A5a): `main.rs` is reduced to the
//! `clap` parse plus `commands::run`, and everything else lives here so
//! `commands::…::handle` is linkable from a `crates/substrate` e2e test --
//! a binary crate cannot be linked as a `dev-dependency`.

pub mod commands;

/// Default API endpoint for the Community Registry.
pub const DEFAULT_API_URL: &str = "http://localhost:7961";
