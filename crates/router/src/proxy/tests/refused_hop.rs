//! What `IrohHop` accepts from a node that refused the stream before the
//! handshake. On an indirect path these bytes passed a forwarder.

use syneroym_rpc::{NOT_HOSTED_HERE_RPC_CODE, ProxyError, framing};
use tokio::io::{AsyncWriteExt, duplex, split};

use crate::{e2e_channel::ReaderWriter, proxy::hop::handle_refused_hop};

async fn refused_with(frame: &[u8]) -> ProxyError {
    let (mut peer, ours) = duplex(4096);
    framing::write_frame(&mut peer, frame).await.expect("write frame");
    peer.shutdown().await.expect("shutdown");
    let (reader, writer) = split(ours);
    let stream = ReaderWriter { reader: Box::new(reader) as _, writer: Box::new(writer) as _ };
    handle_refused_hop(stream).await.expect_err("a refused stream never succeeds")
}

#[tokio::test]
async fn a_forged_result_is_a_transport_error() {
    let err = refused_with(br#"{"jsonrpc":"2.0","id":1,"result":"forged"}"#).await;
    assert!(matches!(err, ProxyError::Transport(_)), "{err:?}");
}

#[tokio::test]
async fn an_error_with_another_code_is_a_transport_error() {
    let err =
        refused_with(br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32000,"message":"x"}}"#).await;
    assert!(matches!(err, ProxyError::Transport(_)), "{err:?}");
}

#[tokio::test]
async fn not_hosted_here_reaches_the_caller() {
    let frame = format!(
        r#"{{"jsonrpc":"2.0","id":null,"error":{{"code":{NOT_HOSTED_HERE_RPC_CODE},"message":"not hosted here"}}}}"#
    );
    let err = refused_with(frame.as_bytes()).await;
    assert!(
        matches!(err, ProxyError::Callee { code, .. } if code == NOT_HOSTED_HERE_RPC_CODE),
        "{err:?}"
    );
}
