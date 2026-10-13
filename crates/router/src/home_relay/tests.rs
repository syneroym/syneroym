use std::{net::SocketAddr, time::Duration};

use iroh::{Endpoint, RelayMap, RelayMode, RelayUrl};
use tokio::{net::TcpListener, time};

use super::{HomeRelay, HomeRelayError};

/// An `http://127.0.0.1:<port>` URL whose listener accepts TCP but never
/// answers, so iroh can never confirm it. Keep the listener alive for the
/// whole test; never reuse a freed port.
async fn dead_relay() -> (TcpListener, RelayUrl) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let url: RelayUrl = format!("http://127.0.0.1:{port}").parse().unwrap();
    (listener, url)
}

async fn bind_empty() -> Endpoint {
    Endpoint::empty_builder()
        .clear_ip_transports()
        .bind_addr(SocketAddr::from(([127, 0, 0, 1], 0)))
        .unwrap()
        .relay_mode(RelayMode::Custom(RelayMap::empty()))
        .bind()
        .await
        .unwrap()
}

#[tokio::test]
async fn first_set_to_a_dead_relay_keeps_it_in_the_map() {
    let ep = bind_empty().await;
    let (_l1, dead1) = dead_relay().await;
    let handle = HomeRelay::new(ep, Duration::from_secs(1));

    let res = handle.set(dead1.clone()).await;
    assert!(matches!(res, Err(HomeRelayError::NotConfirmed { .. })));
    assert_eq!(*handle.subscribe().borrow(), None);
    assert_eq!(handle.in_map().await, Some(dead1));
}

#[tokio::test]
async fn set_after_a_failed_first_set_replaces_the_leftover() {
    let ep = bind_empty().await;
    let (_l1, dead1) = dead_relay().await;
    let (_l2, dead2) = dead_relay().await;
    let handle = HomeRelay::new(ep, Duration::from_secs(1));

    let res1 = handle.set(dead1).await;
    assert!(matches!(res1, Err(HomeRelayError::NotConfirmed { .. })));

    let res2 = handle.set(dead2.clone()).await;
    assert!(matches!(res2, Err(HomeRelayError::NotConfirmed { .. })));
    assert_eq!(handle.in_map().await, Some(dead2));
}

#[tokio::test]
async fn set_on_a_closed_endpoint_is_endpoint_closed() {
    let ep = bind_empty().await;
    let (_l1, dead1) = dead_relay().await;
    let handle = HomeRelay::new(ep.clone(), Duration::from_secs(1));

    ep.close().await;
    let res = handle.set(dead1).await;
    assert!(matches!(res, Err(HomeRelayError::EndpointClosed)));
}

#[tokio::test]
async fn close_during_set_ends_the_wait() {
    let ep = bind_empty().await;
    let (_l1, dead1) = dead_relay().await;
    let handle = HomeRelay::new(ep.clone(), Duration::from_secs(15));

    let start = time::Instant::now();
    let set_task = tokio::spawn(async move { handle.set(dead1).await });

    time::sleep(Duration::from_millis(200)).await;
    ep.close().await;

    let res = set_task.await.unwrap();
    assert!(matches!(res, Err(HomeRelayError::EndpointClosed)));
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "set should return well before 15 s on close: took {:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn clear_unconfirmed_empties_the_map_after_a_failed_first_set() {
    let ep = bind_empty().await;
    let (_l1, dead1) = dead_relay().await;
    let handle = HomeRelay::new(ep, Duration::from_secs(1));

    let res = handle.set(dead1).await;
    assert!(matches!(res, Err(HomeRelayError::NotConfirmed { .. })));

    handle.clear_unconfirmed().await;
    assert_eq!(handle.in_map().await, None);
    assert_eq!(*handle.subscribe().borrow(), None);
}

#[tokio::test]
async fn clear_unconfirmed_on_a_new_handle_does_nothing() {
    let ep = bind_empty().await;
    let handle = HomeRelay::new(ep, Duration::from_secs(1));

    handle.clear_unconfirmed().await;
    assert_eq!(handle.in_map().await, None);
}

#[tokio::test]
async fn closed_returns_after_the_endpoint_closes() {
    let ep = bind_empty().await;
    let handle = HomeRelay::new(ep.clone(), Duration::from_secs(1));

    let task = tokio::spawn(async move { handle.closed().await });
    ep.close().await;

    time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
}
