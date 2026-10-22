use syneroym_rpc::{NOT_HOSTED_HERE_RPC_CODE, framing};

use super::{Refusal, authorized_refusal_frame, write_refusal};
use crate::RouteTransport;

#[test]
fn every_refusal_round_trips_through_its_reason_and_code() {
    for r in Refusal::ALL {
        assert_eq!(Refusal::from_reason(r.reason()), Some(r));
        assert_eq!(Refusal::from_code(r.code()), Some(r));
    }
    assert_eq!(Refusal::from_reason("forged"), None);
    assert_eq!(Refusal::from_code(-32000), None);
}

/// A client rebuilds the refusal from its reason, so the copy must be the
/// exact bytes the router writes.
#[tokio::test]
async fn http_response_is_what_the_router_writes() {
    for r in Refusal::ALL {
        let mut written = Vec::new();
        write_refusal(&mut written, RouteTransport::Http, r).await;
        assert_eq!(written, r.http_response());
    }
}

#[test]
fn a_forged_result_is_not_an_authorized_refusal() {
    let err =
        authorized_refusal_frame(br#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#).unwrap_err();
    assert!(err.contains("carries a result"), "{err}");
}

/// A frame with both keys would pass an error-only parse; a caller that
/// later parsed it as a response would take the `result` as a success.
#[test]
fn a_result_next_to_a_refusal_code_is_not_an_authorized_refusal() {
    let frame = br#"{"jsonrpc":"2.0","id":1,"result":true,"error":{"code":-32096,"message":"x"}}"#;
    let err = authorized_refusal_frame(frame).unwrap_err();
    assert!(err.contains("carries a result"), "{err}");
}

#[test]
fn an_error_with_another_code_is_not_an_authorized_refusal() {
    let frame = br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32000,"message":"x"}}"#;
    let err = authorized_refusal_frame(frame).unwrap_err();
    assert!(err.contains("-32000"), "{err}");
}

#[tokio::test]
async fn every_refusal_the_router_writes_is_authorized() {
    for r in Refusal::ALL {
        let mut written = Vec::new();
        write_refusal(&mut written, RouteTransport::Binary, r).await;
        let frame = framing::read_frame(&mut written.as_slice()).await.expect("frame");
        assert_eq!(authorized_refusal_frame(&frame), Ok(r));
    }
    assert_eq!(Refusal::NotHostedHere.code(), NOT_HOSTED_HERE_RPC_CODE);
}
