//! The clock that every guest call's epoch deadline is measured against.

use std::{thread, time::Duration};

use tokio::sync::oneshot;

use super::EPOCH_TICK_MS;

/// Spawns a dedicated OS thread that calls `tick` every `EPOCH_TICK_MS` until
/// `shutdown_rx` receives a signal or the sender is dropped.
///
/// Running the ticker on a dedicated OS thread ensures that the epoch advances
/// on wall-clock time independently of Tokio worker thread saturation. Guests
/// waiting for a worker thread now use their budget during that wait, matching
/// true wall-clock time.
///
/// Sleep is executed after each tick without catching up on missed ticks.
pub(super) fn spawn(
    mut tick: impl FnMut() + Send + 'static,
    mut shutdown_rx: oneshot::Receiver<()>,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new().name("wasm-epoch-ticker".into()).spawn(move || {
        loop {
            thread::sleep(Duration::from_millis(EPOCH_TICK_MS));
            tick();
            if shutdown_rx.try_recv() != Err(oneshot::error::TryRecvError::Empty) {
                break;
            }
        }
    })
}

#[cfg(test)]
mod tests;
