use syneroym_rpc::{
    NOT_HOSTED_HERE_RPC_CODE, SERVICE_NOT_FOUND_RPC_CODE, STREAM_UNAUTHENTICATED_RPC_CODE, framing,
};
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::{RouteTransport, preamble::REFUSAL_HEADER};

/// Why the router closed a stream before dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    NotHostedHere,
    Unauthenticated,
    UnknownInterface,
}

impl Refusal {
    pub(crate) const fn code(self) -> i32 {
        match self {
            Self::NotHostedHere => NOT_HOSTED_HERE_RPC_CODE,
            Self::Unauthenticated => STREAM_UNAUTHENTICATED_RPC_CODE,
            Self::UnknownInterface => SERVICE_NOT_FOUND_RPC_CODE,
        }
    }

    pub(crate) const fn reason(self) -> &'static str {
        match self {
            Self::NotHostedHere => "not_hosted_here",
            Self::Unauthenticated => "unauthenticated",
            Self::UnknownInterface => "unknown_interface",
        }
    }

    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::NotHostedHere => "not hosted here",
            Self::Unauthenticated => "unauthenticated",
            Self::UnknownInterface => "unknown interface",
        }
    }

    pub(crate) const fn http_status(self) -> &'static str {
        match self {
            Self::NotHostedHere => "421 Misdirected Request",
            Self::Unauthenticated => "401 Unauthorized",
            Self::UnknownInterface => "404 Not Found",
        }
    }

    pub(crate) fn json_body(self) -> Vec<u8> {
        let json = serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {
                "code": self.code(),
                "message": self.message(),
                "data": {
                    "reason": self.reason()
                }
            }
        });
        serde_json::to_vec(&json).unwrap_or_default()
    }
}

/// Writes `refusal` in the framing `transport` uses, flushes, shuts the
/// write half down (QUIC `FIN`), and counts it in
/// `substrate.router.refused{kind}`. It does not log: the caller that
/// decided the refusal logs it once. Write errors are ignored: the peer may
/// already be gone, and the stream ends either way.
pub(crate) async fn write_refusal(
    writer: &mut (impl AsyncWrite + Unpin + Send),
    transport: RouteTransport,
    refusal: Refusal,
) {
    metrics::counter!("substrate.router.refused", "kind" => refusal.reason()).increment(1);

    match transport {
        RouteTransport::Binary => {
            let body = refusal.json_body();
            let _ = framing::write_frame(writer, &body).await;
        }
        RouteTransport::Http => {
            let body = refusal.json_body();
            let status = refusal.http_status();
            let reason = refusal.reason();
            let header = format!(
                "HTTP/1.1 {status}\r\n{REFUSAL_HEADER}: {reason}\r\ncontent-type: \
                 application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            if writer.write_all(header.as_bytes()).await.is_ok() {
                let _ = writer.write_all(&body).await;
            }
        }
        RouteTransport::Raw => {
            let line = format!("{}\n", refusal.reason());
            let _ = writer.write_all(line.as_bytes()).await;
        }
    }
    let _ = writer.flush().await;
    let _ = writer.shutdown().await;
}
