//! Shared stream helpers for substrate end-to-end tests.

use std::time::{SystemTime, UNIX_EPOCH};

use iroh::endpoint::{RecvStream, SendStream};
use syneroym_identity::Identity;
use syneroym_router::{RoutePreamble, RouteProtocol, RouteTransport};
use syneroym_sdk::TransportConnection;

/// Opens a bidirectional HTTP-native stream, sends the route preamble, and
/// signs it if an identity is provided.
pub async fn open_http_stream(
    conn: &TransportConnection,
    service_id: &str,
    identity: Option<&Identity>,
) -> (SendStream, RecvStream) {
    let TransportConnection::Iroh { conn, .. } = conn;
    let (mut send, recv) = conn.open_bi().await.expect("open_bi failed");
    let mut preamble =
        RoutePreamble::new(RouteTransport::Http, RouteProtocol::JsonRpc, service_id, "http-native");
    if let Some(id) = identity {
        let now =
            SystemTime::now().duration_since(UNIX_EPOCH).expect("time went backwards").as_secs();
        preamble.sign(id, now);
    }
    send.write_all(preamble.to_preamble_line().as_bytes()).await.expect("write preamble failed");
    (send, recv)
}
