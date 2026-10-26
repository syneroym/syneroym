use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use iroh::{
    Endpoint, EndpointAddr, RelayMap, RelayMode, SecretKey, TransportAddr,
    endpoint::QuicTransportConfig,
};
use tokio::net::UdpSocket;

use super::*;

const ALPN: &[u8] = b"syneroym-test/close/0";

#[tokio::test(start_paused = true)]
async fn a_close_that_finishes_in_time_returns_true() {
    let done = Arc::new(AtomicBool::new(false));
    let flag = done.clone();
    let finished = close_within("test", Duration::from_secs(3), async move {
        time::sleep(Duration::from_secs(1)).await;
        flag.store(true, Ordering::SeqCst);
    })
    .await;
    assert!(finished);
    assert!(done.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn a_slow_close_returns_at_the_limit_and_keeps_running() {
    let done = Arc::new(AtomicBool::new(false));
    let flag = done.clone();
    let start = time::Instant::now();

    let finished = close_within("test", Duration::from_secs(3), async move {
        time::sleep(Duration::from_secs(39)).await;
        flag.store(true, Ordering::SeqCst);
    })
    .await;

    assert!(!finished, "the close was not done at the limit");
    assert_eq!(start.elapsed(), Duration::from_secs(3), "the caller waited only for the limit");
    assert!(!done.load(Ordering::SeqCst));

    time::sleep(Duration::from_secs(40)).await;
    assert!(done.load(Ordering::SeqCst), "the close finishes in the background");
}

#[tokio::test(start_paused = true)]
async fn a_close_that_panics_does_not_stop_the_caller() {
    let finished =
        close_within("test", Duration::from_secs(3), async { panic!("close failed") }).await;
    assert!(finished, "a failed close is over, and is logged");
}

async fn bind() -> Endpoint {
    Endpoint::empty_builder()
        .clear_ip_transports()
        .bind_addr(SocketAddr::from(([127, 0, 0, 1], 0)))
        .unwrap()
        .relay_mode(RelayMode::Custom(RelayMap::empty()))
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .unwrap()
}

/// A real endpoint with a live connection closes at once, well inside the
/// limit.
#[tokio::test]
async fn a_real_endpoint_with_a_live_peer_closes_within_the_limit() {
    let (a, b) = (bind().await, bind().await);
    let accepting = b.clone();
    tokio::spawn(async move {
        while let Some(incoming) = accepting.accept().await {
            let Ok(conn) = incoming.await else { continue };
            tokio::spawn(async move { conn.closed().await });
        }
    });
    let conn = a.connect(b.addr(), ALPN).await.unwrap();
    let (mut send, _recv) = conn.open_bi().await.unwrap();
    send.write_all(b"hello").await.unwrap();

    let closing = a.clone();
    let start = Instant::now();
    let finished =
        close_within("a", ENDPOINT_CLOSE_LIMIT, async move { closing.close().await }).await;

    assert!(finished);
    assert!(start.elapsed() < ENDPOINT_CLOSE_LIMIT);
    assert!(a.is_closed());
    b.close().await;
}

/// A connection to a peer that never answers has no round-trip sample, so its
/// closing period is three times the probe timeout built from the initial
/// estimate. With an estimate of 5 s that is 45 s, and `Endpoint::close` alone
/// would hold up a shutdown for that long: the same shape as a relay path with
/// a bad estimate. `close_within` must not.
#[tokio::test]
async fn an_endpoint_whose_close_takes_far_longer_than_the_limit_does_not_hold_up_the_caller() {
    let silent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let silent_addr = silent.local_addr().unwrap();
    let a = Endpoint::empty_builder()
        .clear_ip_transports()
        .bind_addr(SocketAddr::from(([127, 0, 0, 1], 0)))
        .unwrap()
        .relay_mode(RelayMode::Custom(RelayMap::empty()))
        .alpns(vec![ALPN.to_vec()])
        .transport_config(
            QuicTransportConfig::builder().initial_rtt(Duration::from_secs(5)).build(),
        )
        .bind()
        .await
        .unwrap();
    let nobody = SecretKey::generate(&mut rand::rng()).public();
    let target = EndpointAddr::from_parts(nobody, [TransportAddr::Ip(silent_addr)]);
    let dialing = a.clone();
    tokio::spawn(async move { dialing.connect(target, ALPN).await });
    // The handshake is under way, and nobody answers it.
    time::sleep(Duration::from_millis(300)).await;

    let limit = Duration::from_millis(500);
    let began = Instant::now();
    let closing = a.clone();
    let finished = close_within("a", limit, async move { closing.close().await }).await;

    assert!(!finished, "the close of this endpoint takes far longer than the limit");
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "the caller waited for the limit, not for the closing period: {:?}",
        began.elapsed()
    );
}
