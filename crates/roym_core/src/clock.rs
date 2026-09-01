//! The one wall-clock read in the product.
//!
//! On `wasm32-wasip2` this lowers to `wasi:clocks/wall-clock`, which the
//! sandbox linker provides; natively it is the process clock. Identical
//! call, identical value source, both builds.
//!
//! Call it at a verb boundary and pass the result down. This clock is
//! *not* the clock the substrate stamps a signed record with: that one
//! belongs to the host and a test can pin it, and this one cannot be
//! pinned from outside a component at all. Anything that must be
//! reproducible across two runs has to derive from the host's stamp or
//! from content, never from here.

use std::time::{SystemTime, UNIX_EPOCH};

/// Unix seconds. A clock before the epoch is impossible on both targets;
/// it saturates to 0 rather than panicking inside a guest.
pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
