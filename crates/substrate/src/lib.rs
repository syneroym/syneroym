//! Main library entry point for the Syneroym substrate.

pub mod identity;
mod runtime;

pub use runtime::{init, init_and_run_with_signal, run, run_with_signal};
