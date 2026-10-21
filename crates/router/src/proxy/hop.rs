use ed25519_dalek::VerifyingKey;
use tokio::io::AsyncWriteExt;

use super::*;
use crate::e2e_channel::{self, Opened};

const REFUSAL_READ_GRACE: Duration = Duration::from_secs(2);

/// Where a remote call goes: the address to dial and the node key that
/// must answer the handshake when that address is not the host itself.
#[derive(Debug, Clone)]
pub struct RemotePeer {
    pub addr: EndpointAddr,
    pub host_key: Option<VerifyingKey>,
}

/// response". The transport-agnostic seam a future wRPC wire (A.5) slots
/// into: a second impl plus a second `ProxyProtocol` variant, nothing else.
#[async_trait::async_trait]
pub trait RemoteHop: Send + Sync + Debug {
    async fn call(
        &self,
        peer: &RemotePeer,
        route: &RoutePreamble,
        identity: Option<&OutboundIdentity>,
        request: &JsonRpcRequest,
        timeout: Duration,
    ) -> Result<Value, ProxyError>;
}

/// [`RemoteHop`] over a live Iroh QUIC connection. `endpoint` is `None` on a
/// WebRTC-only node (no Iroh interface configured) -- every remote hop then
/// fails with a typed transport error rather than panicking.
pub struct IrohHop {
    endpoint: Option<Endpoint>,
    /// Connection-establishment retries only. Forced to a single attempt
    /// (`max_attempts: 1`) regardless of what the caller passes in: the
    /// call-level retry loop in [`ProxyRouter::invoke_remote`] already
    /// retries the whole call (connect + request), so letting
    /// `connect_with_retry` retry underneath it too would multiply worst-
    /// case attempts to `max_attempts²` for an unreachable peer.
    connect_retry_policy: RetryPolicy,
}

impl Debug for IrohHop {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("IrohHop").field("has_endpoint", &self.endpoint.is_some()).finish()
    }
}

impl IrohHop {
    #[must_use]
    pub fn new(endpoint: Option<Endpoint>, retry_policy: RetryPolicy) -> Self {
        Self { endpoint, connect_retry_policy: RetryPolicy { max_attempts: 1, ..retry_policy } }
    }
}

fn transport_err(e: impl std::fmt::Display) -> ProxyError {
    ProxyError::Transport(e.to_string())
}

fn e2e_err(e: e2e_channel::E2eError) -> ProxyError {
    match e {
        e2e_channel::E2eError::HostKeyUnknown { target } => ProxyError::ServiceNotFound(target),
        e @ e2e_channel::E2eError::BadHostSignature { .. } => ProxyError::Callee {
            code: syneroym_rpc::HOST_MISMATCH_RPC_CODE,
            message: e.to_string(),
            data: None,
        },
        other => transport_err(other),
    }
}

#[async_trait::async_trait]
impl RemoteHop for IrohHop {
    async fn call(
        &self,
        peer: &RemotePeer,
        route: &RoutePreamble,
        identity: Option<&OutboundIdentity>,
        request: &JsonRpcRequest,
        timeout: Duration,
    ) -> Result<Value, ProxyError> {
        let endpoint = self
            .endpoint
            .as_ref()
            .ok_or_else(|| ProxyError::Transport("no Iroh endpoint configured".to_string()))?;

        let body = serde_json::to_vec(request)
            .map_err(|e| ProxyError::Internal(format!("failed to serialize request: {e}")))?;

        let preamble = identity
            .map_or_else(|| route.clone(), |id| id.apply(route, crate::handshake::now_secs()));

        let path =
            e2e_channel::path(peer.addr.id.as_bytes(), peer.host_key.as_ref(), &route.service_id)
                .map_err(e2e_err)?;

        // The whole attempt -- connect, open the bi-stream, write the
        // preamble and request, and read the response -- sits under one
        // deadline, not just the final read: a peer that accepts the QUIC
        // connection but stalls before accepting the stream (or stops
        // reading so send-side flow control blocks) must not hang past
        // `timeout`.
        let frame = time::timeout(timeout, async {
            let conn = net_iroh::connect_with_retry(
                endpoint,
                peer.addr.clone(),
                crate::SYNEROYM_ALPN,
                &self.connect_retry_policy,
            )
            .await
            .map_err(transport_err)?;

            let (send, recv) = conn.open_bi().await.map_err(transport_err)?;
            let mut stream = match e2e_channel::open_stream(recv, send, preamble, &path)
                .await
                .map_err(e2e_err)?
            {
                Opened::Refused(mut s) => {
                    return framing::read_frame(&mut s.reader).await.map_err(transport_err);
                }
                Opened::Ready(s) => s,
            };

            let wrote: Result<(), anyhow::Error> = async {
                framing::write_frame(&mut stream.writer, &body).await?;
                stream.writer.shutdown().await?;
                Ok(())
            }
            .await;

            match wrote {
                Ok(()) => framing::read_frame(&mut stream.reader).await.map_err(transport_err),
                // The node may have answered and stopped reading (a refusal before a
                // large request was fully sent). Its answer is still in our receive buffer.
                Err(write_err) => {
                    match time::timeout(REFUSAL_READ_GRACE, framing::read_frame(&mut stream.reader))
                        .await
                    {
                        Ok(Ok(frame)) if !frame.is_empty() => Ok(frame),
                        _ => Err(transport_err(write_err)),
                    }
                }
            }
        })
        .await
        .map_err(|_| ProxyError::Timeout(timeout))??;

        if frame.is_empty() {
            return Err(ProxyError::Transport("empty response frame".to_string()));
        }

        // Success or error envelope -- a JSON-RPC error is a *definitive*
        // answer, never a transport failure. `JsonRpcResponse::result` is a
        // required field, so an error-shaped frame (no `result`) fails this
        // parse and falls through to the error-envelope parse below.
        if let Ok(ok) = serde_json::from_slice::<JsonRpcResponse>(&frame) {
            return Ok(ok.result);
        }
        let err: JsonRpcErrorResponse = serde_json::from_slice(&frame)
            .map_err(|e| ProxyError::Transport(format!("malformed response: {e}")))?;
        Err(ProxyError::Callee {
            code: err.error.code,
            message: err.error.message,
            data: err.error.data,
        })
    }
}
