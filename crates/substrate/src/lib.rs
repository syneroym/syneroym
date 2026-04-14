//! Main library entry point for the Syneroym substrate.

pub mod identity;
mod runtime;

pub use runtime::{run, run_with_signal};
