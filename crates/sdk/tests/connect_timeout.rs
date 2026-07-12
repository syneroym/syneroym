//! `SyneroymClient::connect` must give up on an unreachable peer within its
//! configured deadline rather than hanging indefinitely.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::{
    net::UdpSocket,
    time::{Duration, Instant},
};

use iroh::{EndpointAddr, SecretKey};
use syneroym_core::dht_registry::EndpointMechanism;
use syneroym_sdk::SyneroymClient;

/// A node id pointed at a local UDP port nothing is listening on: iroh has a
/// concrete direct address to dial, but every handshake packet it sends goes
/// unanswered. Without a deadline, `Endpoint::connect` sits in its own
/// hole-punch/handshake retry logic far longer than a caller would ever wait.
///
/// The returned socket must be kept alive for as long as the mechanism is in
/// use: dropping it frees the port, and traffic to a freed port can behave
/// quite differently (e.g. an immediate refusal) from traffic to one that is
/// bound but silent, which is the scenario under test.
fn unreachable_mechanism() -> (EndpointMechanism, UdpSocket) {
    // Binding (rather than picking an arbitrary port) guarantees the port is
    // real and currently unused, without guessing at one that might collide.
    let black_hole = UdpSocket::bind("127.0.0.1:0").expect("bind black-hole socket");
    let black_hole_addr = black_hole.local_addr().expect("local_addr");

    let bogus_node_id = SecretKey::generate(&mut rand::rng()).public();
    let endpoint_addr = EndpointAddr::new(bogus_node_id).with_ip_addr(black_hole_addr);
    let mechanism = EndpointMechanism::Iroh {
        endpoint_addr_bytes: serde_json::to_vec(&endpoint_addr).expect("serialize EndpointAddr"),
        relay_url: None,
    };
    (mechanism, black_hole)
}

#[tokio::test]
async fn connect_gives_up_on_unreachable_peer_within_deadline() {
    let deadline = Duration::from_millis(300);
    let (mechanism, _black_hole) = unreachable_mechanism();
    let mut client =
        SyneroymClient::new_with_mechanisms("unreachable-peer".to_string(), vec![mechanism])
            .with_connect_timeout(deadline);

    let start = Instant::now();
    let result = client.connect().await;
    let elapsed = start.elapsed();

    let err = result.expect_err("connect to an unreachable peer must fail, not succeed");
    assert!(err.to_string().contains("timed out"), "expected a timeout error, got: {err}");
    // Slack over `deadline` for CI scheduling jitter, but tight enough to
    // catch a regression to the ~3s `Endpoint::close` grace period leaking
    // onto this call's return time (see `close_in_background` in the SDK).
    assert!(
        elapsed < Duration::from_secs(2),
        "connect took {elapsed:?}, expected it to give up near the {deadline:?} deadline"
    );
}
