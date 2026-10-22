use syneroym_rpc::{
    JsonRpcErrorResponse, NOT_HOSTED_HERE_RPC_CODE, SERVICE_NOT_FOUND_RPC_CODE,
    STREAM_UNAUTHENTICATED_RPC_CODE, framing,
};
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::{RouteTransport, preamble::REFUSAL_HEADER};

/// Why the router closed a stream before dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    NotHostedHere,
    Unauthenticated,
    UnknownInterface,
}

impl Refusal {
    const ALL: [Self; 3] = [Self::NotHostedHere, Self::Unauthenticated, Self::UnknownInterface];

    /// The refusal a `REFUSAL_HEADER` value names, or `None` for any other
    /// value.
    #[must_use]
    pub fn from_reason(reason: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.reason() == reason)
    }

    /// The refusal a JSON-RPC error code names, or `None` for any other
    /// code.
    #[must_use]
    pub fn from_code(code: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.code() == code)
    }

    /// The whole HTTP response the router writes for this refusal. A client
    /// that receives a refusal through a forwarder shows this copy, never
    /// the bytes it received.
    #[must_use]
    pub fn http_response(self) -> Vec<u8> {
        let body = self.json_body();
        let header = format!(
            "HTTP/1.1 {}\r\n{REFUSAL_HEADER}: {}\r\ncontent-type: \
             application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            self.http_status(),
            self.reason(),
            body.len()
        );
        [header.into_bytes(), body].concat()
    }

    pub(crate) const fn code(self) -> i32 {
        match self {
            Self::NotHostedHere => NOT_HOSTED_HERE_RPC_CODE,
            Self::Unauthenticated => STREAM_UNAUTHENTICATED_RPC_CODE,
            Self::UnknownInterface => SERVICE_NOT_FOUND_RPC_CODE,
        }
    }

    #[must_use]
    pub const fn reason(self) -> &'static str {
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

/// Reads the binary-framed answer to a stream the node refused before the
/// handshake. It is accepted only as a JSON-RPC error with a refusal code:
/// on an indirect path the answer passed a forwarder, which could
/// otherwise forge a result or any other error.
///
/// # Errors
/// A message naming why the frame is not an authorized refusal.
pub fn authorized_refusal_frame(frame: &[u8]) -> Result<JsonRpcErrorResponse, String> {
    let err: JsonRpcErrorResponse = serde_json::from_slice(frame)
        .map_err(|e| format!("refusal response is not a valid JSON-RPC error: {e}"))?;
    if Refusal::from_code(err.error.code).is_none() {
        return Err(format!(
            "refusal response code {} is not an authorized pre-handshake error",
            err.error.code
        ));
    }
    Ok(err)
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
            let _ = writer.write_all(&refusal.http_response()).await;
        }
        RouteTransport::Raw => {
            let line = format!("{}\n", refusal.reason());
            let _ = writer.write_all(line.as_bytes()).await;
        }
    }
    let _ = writer.flush().await;
    let _ = writer.shutdown().await;
}

#[cfg(test)]
mod tests;
