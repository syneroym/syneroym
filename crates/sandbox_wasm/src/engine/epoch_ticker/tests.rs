use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use tokio::time;

use super::*;

/// Starts a ticker that counts its ticks, returning the counter, the shutdown
/// sender, and the thread handle.
fn counting_ticker() -> (Arc<AtomicU64>, oneshot::Sender<()>, thread::JoinHandle<()>) {
    let ticks = Arc::new(AtomicU64::new(0));
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let counter = ticks.clone();
    let handle = spawn(move || _ = counter.fetch_add(1, Ordering::SeqCst), shutdown_rx)
        .expect("spawn ticker thread");
    (ticks, shutdown_tx, handle)
}

#[tokio::test]
async fn the_ticker_advances_independently_of_worker_threads() {
    let (ticks, shutdown, handle) = counting_ticker();
    let before = ticks.load(Ordering::SeqCst);

    // Synchronously block the tokio worker thread for 5 ticks. The dedicated
    // OS ticker thread must continue ticking.
    thread::sleep(Duration::from_millis(5 * EPOCH_TICK_MS));
    let after = ticks.load(Ordering::SeqCst);

    assert!(
        after >= before + 3,
        "ticker should have advanced while runtime thread was blocked: before={}, after={}",
        before,
        after
    );

    drop(shutdown);
    handle.join().expect("join ticker thread");
}

#[tokio::test]
async fn the_ticker_stops_at_shutdown() {
    let (ticks, shutdown, handle) = counting_ticker();
    time::sleep(Duration::from_millis(EPOCH_TICK_MS / 2)).await;

    shutdown.send(()).expect("send shutdown signal");
    handle.join().expect("join ticker thread");
    let stopped_at = ticks.load(Ordering::SeqCst);

    time::sleep(Duration::from_millis(3 * EPOCH_TICK_MS)).await;
    assert_eq!(ticks.load(Ordering::SeqCst), stopped_at);
}

#[tokio::test]
async fn dropping_the_sender_ends_the_ticker_thread() {
    let (_ticks, shutdown, handle) = counting_ticker();

    drop(shutdown);

    let (done_tx, done_rx) = oneshot::channel();
    thread::spawn(move || {
        let _ = handle.join();
        let _ = done_tx.send(());
    });

    time::timeout(Duration::from_millis(500), done_rx)
        .await
        .expect("ticker thread did not end after sender was dropped")
        .expect("receive termination signal");
}
