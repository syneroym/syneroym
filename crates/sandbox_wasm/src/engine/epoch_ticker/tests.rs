use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use super::*;

/// Starts a ticker that counts its ticks, and lets it take the first one.
async fn counting_ticker() -> (Arc<AtomicU64>, oneshot::Sender<()>) {
    let ticks = Arc::new(AtomicU64::new(0));
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let counter = ticks.clone();
    spawn(move || _ = counter.fetch_add(1, Ordering::SeqCst), shutdown_rx);
    time::sleep(Duration::from_millis(EPOCH_TICK_MS / 2)).await;
    (ticks, shutdown_tx)
}

#[tokio::test]
async fn a_blocked_runtime_does_not_make_up_the_ticks_it_missed() {
    let (ticks, _shutdown) = counting_ticker().await;
    let before = ticks.load(Ordering::SeqCst);

    // The only runtime thread is blocked for ten ticks, as a slow
    // synchronous call inside an async task would block it.
    thread::sleep(Duration::from_millis(10 * EPOCH_TICK_MS));
    time::sleep(Duration::from_millis(EPOCH_TICK_MS / 2)).await;

    let after = ticks.load(Ordering::SeqCst);
    assert!(after - before <= 2, "the ticker made up {} missed ticks", after - before);
}

#[tokio::test]
async fn the_ticker_stops_at_shutdown() {
    let (ticks, shutdown) = counting_ticker().await;
    shutdown.send(()).unwrap();
    time::sleep(Duration::from_millis(EPOCH_TICK_MS / 2)).await;
    let stopped_at = ticks.load(Ordering::SeqCst);

    time::sleep(Duration::from_millis(3 * EPOCH_TICK_MS)).await;

    assert_eq!(ticks.load(Ordering::SeqCst), stopped_at);
}
