//! What the gateway passthrough sends its HTTP client when the node
//! refused the stream before the handshake. On an indirect path the
//! refusal passed a forwarder, so only an authorized refusal head is
//! accepted, and the client gets the router's own copy of it.

use syneroym_router::{OwnedStream, ReaderWriter, Refusal};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, duplex, split},
    net::{TcpListener, TcpStream},
};

use super::{PassthroughOutcome, handle_refused_passthrough};

/// Runs `handle_refused_passthrough` on a stream whose peer writes
/// `chunks`, and returns its outcome and what the TCP client received.
async fn refuse_with(chunks: &[&[u8]]) -> (anyhow::Result<PassthroughOutcome>, Vec<u8>) {
    let (mut peer, ours) = duplex(8192);
    for chunk in chunks {
        peer.write_all(chunk).await.expect("write");
    }
    peer.shutdown().await.expect("shutdown");
    let (reader, writer) = split(ours);
    let stream: OwnedStream = ReaderWriter { reader: Box::new(reader), writer: Box::new(writer) };

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let mut client = TcpStream::connect(listener.local_addr().expect("addr")).await.expect("dial");
    let (mut server_side, _) = listener.accept().await.expect("accept");

    let outcome = handle_refused_passthrough(stream, &mut server_side).await;
    drop(server_side);
    let mut received = Vec::new();
    client.read_to_end(&mut received).await.expect("read");
    (outcome, received)
}

#[tokio::test]
async fn a_forged_success_is_refused_and_nothing_reaches_the_client() {
    let (outcome, received) =
        refuse_with(&[b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok"]).await;
    assert!(outcome.is_err());
    assert!(received.is_empty(), "client got {:?}", String::from_utf8_lossy(&received));
}

#[tokio::test]
async fn an_error_status_without_a_known_reason_is_refused() {
    let (outcome, received) =
        refuse_with(&[b"HTTP/1.1 401 Unauthorized\r\nx-syneroym-refusal: forged\r\n\r\n"]).await;
    assert!(outcome.is_err());
    assert!(received.is_empty());
}

/// A forwarder may keep a real refusal head and add its own body. The
/// client must see the router's copy, not that body.
#[tokio::test]
async fn the_client_gets_the_routers_copy_not_the_received_body() {
    let (outcome, received) = refuse_with(&[
        b"HTTP/1.1 401 Unauthorized\r\nx-syneroym-refusal: unauth",
        b"enticated\r\n\r\n<script>forged</script>",
    ])
    .await;
    assert_eq!(outcome.expect("handled"), PassthroughOutcome::Done);
    assert_eq!(received, Refusal::Unauthenticated.http_response());
}

#[tokio::test]
async fn not_hosted_here_is_replayable_and_sends_nothing() {
    let (outcome, received) = refuse_with(&[&Refusal::NotHostedHere.http_response()]).await;
    assert_eq!(outcome.expect("handled"), PassthroughOutcome::NotHostedHere { replayable: true });
    assert!(received.is_empty());
}

#[tokio::test]
async fn a_head_that_never_ends_is_refused() {
    let long = [b'a'; 5000];
    let (outcome, received) = refuse_with(&[b"HTTP/1.1 401 Unauthorized\r\n", &long]).await;
    assert!(outcome.is_err());
    assert!(received.is_empty());
}
