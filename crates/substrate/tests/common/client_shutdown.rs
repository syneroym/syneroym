//! Shared `shutdown_clients` helper for `crates/substrate/tests/*.rs`
//! integration suites that hold a batch of `Arc<SyneroymClient>` (e.g. one
//! per peer alias) rather than a single client owned by a `Node`. Kept
//! separate from `common/mod.rs`'s `SubstrateTestContext` bootstrap, the
//! same reason `common/retry.rs` is split out: a consuming file that only
//! needs this one helper shouldn't have to pull in (and trip dead-code
//! warnings on) that unrelated struct.

use std::sync::Arc;

use syneroym_sdk::SyneroymClient;

/// Closes each client's iroh endpoint explicitly instead of leaving it to
/// `Drop`'s fire-and-forget safety net -- a client dropped that way races
/// this test's own tokio runtime shutdown and can trip iroh's "Endpoint
/// dropped without calling `Endpoint::close`" warning. Only closes a client
/// this call holds the sole `Arc` to; if something else still references
/// it, leaving it open is correct, not a leak.
#[allow(dead_code)]
pub async fn shutdown_clients(clients: impl IntoIterator<Item = Arc<SyneroymClient>>) {
    for mut client in clients {
        if let Some(c) = Arc::get_mut(&mut client) {
            let _ = c.shutdown().await;
        }
    }
}
