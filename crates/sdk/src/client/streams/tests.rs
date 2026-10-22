//! What the SDK request path reports for a stream the node refused before
//! the handshake. On an indirect path the refusal passed a forwarder.

use syneroym_router::{OwnedStream, ReaderWriter, Refusal};
use syneroym_rpc::{JsonRpcError, NOT_HOSTED_HERE_RPC_CODE, framing};
use tokio::io::{AsyncWriteExt, duplex, split};

use super::refusal_error;

async fn refused_with(frame: &[u8]) -> anyhow::Error {
    let (mut peer, ours) = duplex(4096);
    framing::write_frame(&mut peer, frame).await.expect("write frame");
    peer.shutdown().await.expect("shutdown");
    let (reader, writer) = split(ours);
    let stream: OwnedStream = ReaderWriter { reader: Box::new(reader), writer: Box::new(writer) };
    refusal_error(stream).await
}

/// One frame can carry both keys. A later parse must never get the
/// chance to read the forged `result` as a success.
#[tokio::test]
async fn a_frame_with_a_result_and_a_refusal_code_is_not_a_refusal() {
    let err = refused_with(
        br#"{"jsonrpc":"2.0","id":1,"result":{"accepted":true},"error":{"code":-32096,"message":"x"}}"#,
    )
    .await;
    assert!(err.downcast_ref::<JsonRpcError>().is_none(), "{err}");
    assert!(err.to_string().contains("carries a result"), "{err}");
}

#[tokio::test]
async fn a_refusal_is_reported_with_the_routers_own_copy() {
    let frame = format!(
        r#"{{"jsonrpc":"2.0","id":null,"error":{{"code":{NOT_HOSTED_HERE_RPC_CODE},"message":"forged text","data":{{"forged":true}}}}}}"#
    );
    let err = refused_with(frame.as_bytes()).await;
    let rpc = err.downcast_ref::<JsonRpcError>().expect("a JSON-RPC error");
    let own = Refusal::NotHostedHere.json_rpc_error();
    assert_eq!((rpc.code, &rpc.message, &rpc.data), (own.code, &own.message, &own.data));
}
