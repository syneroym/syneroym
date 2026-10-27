//! Runtime tests for Iroh endpoint relay map and HomeRelay behavior.
//!
//! Verifies dynamic relay insertion, removal, home relay selection, rollback,
//! and relay-assisted connectivity for endpoints configured with empty relay
//! maps.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayConfig, RelayMap, RelayMode, RelayUrl, TransportAddr,
};
use iroh_relay::server::{RelayConfig as ServerRelayConfig, Server, ServerConfig};
use syneroym_router::{HOME_RELAY_CONFIRM_TIMEOUT, HomeRelay, HomeRelayError, net_iroh};
use tokio::{net::TcpListener, task::JoinHandle, time};

const ALPN: &[u8] = b"syneroym-test/relay-map/0";
const WAIT: Duration = Duration::from_secs(10);

/// Spawns a plain-HTTP relay server on a dynamic local port.
///
/// Keeps the relay server instance alive and returns its HTTP relay URL.
async fn spawn_relay() -> Result<(Server, RelayUrl)> {
    let http_bind_addr = SocketAddr::from(([127, 0, 0, 1], 0));
    let mut relay_config = ServerRelayConfig::new(http_bind_addr);
    relay_config.key_cache_capacity = Some(1024);
    let mut server_config = ServerConfig::default();
    server_config.relay = Some(relay_config);
    let server = Server::spawn(server_config).await.context("failed to spawn relay server")?;
    let http_addr = server.http_addr().context("relay server has no HTTP address")?;
    let url: RelayUrl =
        format!("http://{http_addr}").parse().context("failed to parse relay url")?;
    Ok((server, url))
}

/// Binds an endpoint with an empty relay map and local loopback binding.
async fn bind_empty() -> Result<Endpoint> {
    time::timeout(WAIT, async {
        let ep = net_iroh::empty_endpoint_builder()
            .clear_ip_transports()
            .bind_addr(SocketAddr::from(([127, 0, 0, 1], 0)))
            .context("failed to bind loopback address")?
            .relay_mode(RelayMode::Custom(RelayMap::empty()))
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .context("failed to bind endpoint")?;
        Ok(ep)
    })
    .await
    .context("bind_empty timed out")?
}

/// Polls `ep.addr().relay_urls()` every 50 ms until `pred` is true or WAIT
/// passes. Returns the last observed list of relay URLs.
async fn wait_for_relays(ep: &Endpoint, pred: impl Fn(&[RelayUrl]) -> bool) -> Vec<RelayUrl> {
    let start = time::Instant::now();
    loop {
        let relays: Vec<RelayUrl> = ep.addr().relay_urls().cloned().collect();
        if pred(&relays) {
            return relays;
        }
        if start.elapsed() >= WAIT {
            return relays;
        }
        time::sleep(Duration::from_millis(50)).await;
    }
}

/// Accepts one connection on `ep` and echoes one bi-directional stream.
fn spawn_echo(ep: Endpoint) -> JoinHandle<Result<()>> {
    tokio::spawn(async move {
        time::timeout(WAIT, async {
            let incoming = ep.accept().await.context("endpoint closed before connection")?;
            let conn = incoming.await.context("failed to accept connection")?;
            let (mut send, mut recv) =
                conn.accept_bi().await.context("failed to accept bi stream")?;
            let data = recv.read_to_end(1000).await.context("failed to read from stream")?;
            send.write_all(&data).await.context("failed to write echo to stream")?;
            send.finish().context("failed to finish send stream")?;
            conn.closed().await;
            Ok(())
        })
        .await
        .context("spawn_echo timed out")?
    })
}

/// Dials `target` using an address that contains only the relay URL,
/// sends a ping, and asserts the echoed response matches.
async fn dial_via_relay_only(from: &Endpoint, target: EndpointId, relay: &RelayUrl) -> Result<()> {
    time::timeout(WAIT, async {
        let addr = EndpointAddr::from_parts(target, [TransportAddr::Relay(relay.clone())]);
        let conn = from.connect(addr, ALPN).await.context("failed to connect via relay")?;
        let (mut send, mut recv) = conn.open_bi().await.context("failed to open bi stream")?;
        send.write_all(b"ping").await.context("failed to send ping")?;
        send.finish().context("failed to finish send stream")?;
        let resp = recv.read_to_end(1000).await.context("failed to read ping response")?;
        ensure!(resp == b"ping", "expected ping response, got {resp:?}");
        conn.close(0u32.into(), b"done");
        conn.closed().await;
        Ok(())
    })
    .await
    .context("dial_via_relay_only timed out")?
}

#[tokio::test]
async fn empty_relay_map_has_no_home_relay() -> Result<()> {
    let ep = bind_empty().await?;
    let relays: Vec<RelayUrl> = ep.addr().relay_urls().cloned().collect();
    assert!(relays.is_empty(), "expected empty relay urls, got {relays:?}");

    let online_res = time::timeout(Duration::from_secs(2), ep.online()).await;
    assert!(online_res.is_err(), "expected online() to time out with empty relay map");

    ep.close().await;
    Ok(())
}

#[tokio::test]
async fn relay_inserted_after_bind_becomes_home_relay() -> Result<()> {
    let (_relay_server, r1) = spawn_relay().await?;
    let a = bind_empty().await?;

    let insert_start = time::Instant::now();
    a.insert_relay(r1.clone(), Arc::new(RelayConfig::new(r1.clone(), None))).await;

    let observed = wait_for_relays(&a, |r| r == [r1.clone()]).await;
    let elapsed = insert_start.elapsed();
    eprintln!("Time from insert to home relay listed: {elapsed:?}");
    assert_eq!(observed, vec![r1.clone()]);

    let online_res = time::timeout(WAIT, a.online()).await;
    assert!(online_res.is_ok(), "expected online() to finish within WAIT");

    let b = bind_empty().await?;
    let echo_handle = spawn_echo(a.clone());
    dial_via_relay_only(&b, a.id(), &r1).await?;
    echo_handle.await??;

    a.close().await;
    b.close().await;
    Ok(())
}

#[tokio::test]
async fn removing_home_relay_and_inserting_another_moves_home_relay() -> Result<()> {
    let (_relay_server_1, r1) = spawn_relay().await?;
    let (_relay_server_2, r2) = spawn_relay().await?;
    let a = bind_empty().await?;

    a.insert_relay(r1.clone(), Arc::new(RelayConfig::new(r1.clone(), None))).await;
    let observed = wait_for_relays(&a, |r| r == [r1.clone()]).await;
    assert_eq!(observed, vec![r1.clone()]);

    a.remove_relay(&r1).await;
    a.insert_relay(r2.clone(), Arc::new(RelayConfig::new(r2.clone(), None))).await;
    let observed_r2 = wait_for_relays(&a, |r| r == [r2.clone()]).await;
    eprintln!("Relay list after inserting second relay: {observed_r2:?}");
    assert_eq!(observed_r2, vec![r2.clone()]);

    let b = bind_empty().await?;
    let echo_handle = spawn_echo(a.clone());
    dial_via_relay_only(&b, a.id(), &r2).await?;
    echo_handle.await??;

    a.close().await;
    b.close().await;
    Ok(())
}

#[tokio::test]
async fn endpoint_without_home_relay_dials_peer_through_peer_relay() -> Result<()> {
    let (_relay_server, r1) = spawn_relay().await?;
    let a = bind_empty().await?;

    a.insert_relay(r1.clone(), Arc::new(RelayConfig::new(r1.clone(), None))).await;
    let observed = wait_for_relays(&a, |r| r == [r1.clone()]).await;
    assert_eq!(observed, vec![r1.clone()]);

    let echo_handle = spawn_echo(a.clone());
    let b = bind_empty().await?;
    dial_via_relay_only(&b, a.id(), &r1).await?;
    echo_handle.await??;

    a.close().await;
    b.close().await;
    Ok(())
}

#[tokio::test]
async fn two_relays_in_map_home_relay_list() -> Result<()> {
    let (_relay_server_1, r1) = spawn_relay().await?;
    let (_relay_server_2, r2) = spawn_relay().await?;
    let a = bind_empty().await?;

    a.insert_relay(r1.clone(), Arc::new(RelayConfig::new(r1.clone(), None))).await;
    let observed_r1 = wait_for_relays(&a, |r| r == [r1.clone()]).await;
    assert_eq!(observed_r1, vec![r1.clone()]);

    a.insert_relay(r2.clone(), Arc::new(RelayConfig::new(r2.clone(), None))).await;

    // Allow time for the network report and address watcher to settle.
    time::sleep(Duration::from_millis(500)).await;
    let final_list: Vec<RelayUrl> = a.addr().relay_urls().cloned().collect();
    eprintln!("Final relay list with two relays in map: {final_list:?}");

    assert!(!final_list.is_empty(), "expected relay list not to be empty");
    for url in &final_list {
        assert!(url == &r1 || url == &r2, "unexpected relay url in list: {url}");
    }

    a.close().await;
    Ok(())
}

/// A listener on 127.0.0.1:0 that accepts TCP but never answers.
async fn dead_relay() -> Result<(TcpListener, RelayUrl)> {
    let listener = TcpListener::bind("127.0.0.1:0").await.context("failed to bind dead relay")?;
    let port = listener.local_addr()?.port();
    let url: RelayUrl =
        format!("http://127.0.0.1:{port}").parse().context("parse dead relay url")?;
    Ok((listener, url))
}

#[tokio::test]
async fn inserting_new_relay_then_removing_old_moves_home_relay() -> Result<()> {
    let (_relay_server_1, r1) = spawn_relay().await?;
    let (_relay_server_2, r2) = spawn_relay().await?;
    let a = bind_empty().await?;

    a.insert_relay(r1.clone(), Arc::new(RelayConfig::new(r1.clone(), None))).await;
    let observed_r1 = wait_for_relays(&a, |r| r == [r1.clone()]).await;
    assert_eq!(observed_r1, vec![r1.clone()]);

    a.insert_relay(r2.clone(), Arc::new(RelayConfig::new(r2.clone(), None))).await;
    a.remove_relay(&r1).await;

    let observed_r2 = wait_for_relays(&a, |r| r == [r2.clone()]).await;
    assert_eq!(observed_r2, vec![r2.clone()]);

    let b = bind_empty().await?;
    let echo_handle = spawn_echo(a.clone());
    dial_via_relay_only(&b, a.id(), &r2).await?;
    echo_handle.await??;

    a.close().await;
    b.close().await;
    Ok(())
}

#[tokio::test]
async fn home_relay_set_confirms_relay_and_updates_watch() -> Result<()> {
    let (_relay_server_1, r1) = spawn_relay().await?;
    let a = bind_empty().await?;
    let handle = HomeRelay::new(a.clone(), HOME_RELAY_CONFIRM_TIMEOUT);
    let mut rx = handle.subscribe();

    handle.set(r1.clone()).await?;
    assert!(rx.has_changed()?);
    assert_eq!(*rx.borrow_and_update(), Some(r1.clone()));

    let relays: Vec<RelayUrl> = a.addr().relay_urls().cloned().collect();
    assert_eq!(relays, vec![r1.clone()]);

    let b = bind_empty().await?;
    let echo_handle = spawn_echo(a.clone());
    dial_via_relay_only(&b, a.id(), &r1).await?;
    echo_handle.await??;

    handle.set(r1.clone()).await?;
    assert!(!rx.has_changed()?);

    a.close().await;
    b.close().await;
    Ok(())
}

#[tokio::test]
async fn home_relay_switch_moves_home_relay_and_updates_watch() -> Result<()> {
    let (_relay_server_1, r1) = spawn_relay().await?;
    let (_relay_server_2, r2) = spawn_relay().await?;
    let a = bind_empty().await?;
    let handle = HomeRelay::new(a.clone(), HOME_RELAY_CONFIRM_TIMEOUT);
    let mut rx = handle.subscribe();

    let start_1 = time::Instant::now();
    handle.set(r1.clone()).await?;
    let elapsed_1 = start_1.elapsed();
    eprintln!("First set(r1) confirm time: {elapsed_1:?}");

    let start_2 = time::Instant::now();
    handle.set(r2.clone()).await?;
    let elapsed_2 = start_2.elapsed();
    eprintln!("Switch set(r2) confirm time: {elapsed_2:?}");

    assert_eq!(*rx.borrow_and_update(), Some(r2.clone()));
    let relays: Vec<RelayUrl> = a.addr().relay_urls().cloned().collect();
    assert_eq!(relays, vec![r2.clone()]);

    let b = bind_empty().await?;
    let echo_handle = spawn_echo(a.clone());
    dial_via_relay_only(&b, a.id(), &r2).await?;
    echo_handle.await??;

    a.close().await;
    b.close().await;
    Ok(())
}

#[tokio::test]
async fn home_relay_switch_to_a_dead_relay_rolls_back() -> Result<()> {
    let (_relay_server_1, r1) = spawn_relay().await?;
    let (_dead_listener, dead) = dead_relay().await?;
    let a = bind_empty().await?;
    let handle = HomeRelay::new(a.clone(), HOME_RELAY_CONFIRM_TIMEOUT);
    let mut rx = handle.subscribe();

    handle.set(r1.clone()).await?;
    assert_eq!(*rx.borrow_and_update(), Some(r1.clone()));

    let res = handle.set(dead).await;
    assert!(matches!(res, Err(HomeRelayError::NotConfirmed { .. })));
    assert_eq!(*rx.borrow_and_update(), Some(r1.clone()));

    let relays: Vec<RelayUrl> = a.addr().relay_urls().cloned().collect();
    assert_eq!(relays, vec![r1.clone()]);

    let b = bind_empty().await?;
    let echo_handle = spawn_echo(a.clone());
    dial_via_relay_only(&b, a.id(), &r1).await?;
    echo_handle.await??;

    a.close().await;
    b.close().await;
    Ok(())
}

#[tokio::test]
async fn home_relay_set_after_a_rolled_back_switch_works() -> Result<()> {
    let (_relay_server_1, r1) = spawn_relay().await?;
    let (_relay_server_2, r2) = spawn_relay().await?;
    let (_dead_listener, dead) = dead_relay().await?;
    let a = bind_empty().await?;
    let handle = HomeRelay::new(a.clone(), HOME_RELAY_CONFIRM_TIMEOUT);
    let mut rx = handle.subscribe();

    handle.set(r1.clone()).await?;
    let res = handle.set(dead).await;
    assert!(matches!(res, Err(HomeRelayError::NotConfirmed { .. })));

    handle.set(r2.clone()).await?;
    assert_eq!(*rx.borrow_and_update(), Some(r2.clone()));

    let relays: Vec<RelayUrl> = a.addr().relay_urls().cloned().collect();
    assert_eq!(relays, vec![r2.clone()]);

    let b = bind_empty().await?;
    let echo_handle = spawn_echo(a.clone());
    dial_via_relay_only(&b, a.id(), &r2).await?;
    echo_handle.await??;

    a.close().await;
    b.close().await;
    Ok(())
}
