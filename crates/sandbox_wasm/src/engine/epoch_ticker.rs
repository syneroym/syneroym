//! The clock that every guest call's epoch deadline is measured against.

use std::time::Duration;

use tokio::{
    sync::oneshot,
    time::{self, MissedTickBehavior},
};

use super::EPOCH_TICK_MS;

/// Spawns the task that calls `tick` every `EPOCH_TICK_MS` until
/// `shutdown_rx` fires.
///
/// A deadline is the epoch at the start of a call plus a number of ticks.
/// When the runtime was blocked, the ticks it missed are not made up later:
/// a burst of them would end a call that just started, as soon as it awaits
/// a host call. The epoch only counts time the ticker could see.
pub(super) fn spawn(
    mut tick: impl FnMut() + Send + 'static,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_millis(EPOCH_TICK_MS));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = interval.tick() => tick(),
                _ = &mut shutdown_rx => break,
            }
        }
    });
}

#[cfg(test)]
mod tests;
