use std::time::Duration;

use anyhow::{Context, Result};
use syneroym_router::{Opened, OwnedStream, RoutePreamble, authorized_refusal_frame, e2e_channel};
use syneroym_rpc::{JsonRpcErrorResponse, JsonRpcRequest, JsonRpcResponse, framing};
use tokio::{io::AsyncWriteExt, time};

use super::{SyneroymClient, TransportConnection, check_frame_size, now_secs};

pub(crate) const REFUSAL_READ_GRACE: Duration = Duration::from_secs(2);

/// What happened after the request was written.
pub(crate) enum Sent {
    /// The stream is open; the answer comes on `stream.reader`.
    Open(OwnedStream),
    /// The node answered before the request was fully sent (a refusal
    /// before the handshake, or one read after a failed write).
    Answered(Vec<u8>),
}

impl SyneroymClient {
    pub(crate) fn build_preamble(&self, interface_name: &str) -> RoutePreamble {
        let mut preamble = RoutePreamble::binary_json_rpc(&self.service_id, interface_name);
        preamble.ucan = self.caller_ucan.clone();
        preamble.delegation = self.caller_delegation.clone();
        preamble.sign(&self.identity, now_secs());
        preamble
    }

    /// Opens a stream, writes the preamble (and, through a forwarder, runs
    /// the handshake and sends the identity line), then writes one request
    /// frame. `finish` closes the send half after the frame.
    async fn send_request(
        &self,
        conn: &TransportConnection,
        interface: &str,
        req_bytes: &[u8],
        finish: bool,
    ) -> Result<Sent> {
        let TransportConnection::Iroh { conn, host_key, .. } = conn;
        let path =
            e2e_channel::path(conn.remote_id().as_bytes(), host_key.as_ref(), &self.service_id)?;
        let (send, recv) = conn.open_bi().await?;
        let mut stream =
            match e2e_channel::open_stream(recv, send, self.build_preamble(interface), &path)
                .await?
            {
                Opened::Refused(mut s) => {
                    let frame = time::timeout(
                        e2e_channel::HANDSHAKE_TIMEOUT,
                        framing::read_frame(&mut s.reader),
                    )
                    .await
                    .map_err(|_| anyhow::anyhow!("timeout waiting for refusal response"))??;

                    authorized_refusal_frame(&frame).map_err(anyhow::Error::msg)?;
                    return Ok(Sent::Answered(frame));
                }
                Opened::Ready(s) => s,
            };
        let wrote: Result<()> = async {
            framing::write_frame(&mut stream.writer, req_bytes).await?;
            if finish {
                stream.writer.shutdown().await?;
            }
            Ok(())
        }
        .await;
        match wrote {
            Ok(()) => Ok(Sent::Open(stream)),
            Err(write_err) => {
                match time::timeout(REFUSAL_READ_GRACE, framing::read_frame(&mut stream.reader))
                    .await
                {
                    Ok(Ok(f)) if !f.is_empty() => Ok(Sent::Answered(f)),
                    _ => Err(write_err),
                }
            }
        }
    }

    pub(crate) async fn request_once(
        &self,
        interface_name: &str,
        request: &JsonRpcRequest,
    ) -> (Result<JsonRpcResponse>, Option<TransportConnection>) {
        let req_bytes = match serde_json::to_vec(request) {
            Ok(b) => b,
            Err(e) => return (Err(e.into()), None),
        };
        if let Err(e) = check_frame_size(&request.method, &req_bytes) {
            return (Err(e), None);
        }

        let conn_wrapper = match self.connection() {
            Some(c) => c,
            None => return (Err(anyhow::anyhow!("Not connected")), None),
        };

        let sent = match self.send_request(&conn_wrapper, interface_name, &req_bytes, true).await {
            Ok(s) => s,
            Err(e) => return (Err(e), Some(conn_wrapper)),
        };

        let frame = match sent {
            Sent::Open(mut s) => match framing::read_frame(&mut s.reader).await {
                Ok(f) => f,
                Err(e) => return (Err(e), Some(conn_wrapper)),
            },
            Sent::Answered(f) => f,
        };

        if frame.is_empty() {
            return (
                Err(anyhow::anyhow!("Empty response from stream for method {}", request.method)),
                Some(conn_wrapper),
            );
        }

        let res: JsonRpcResponse = match serde_json::from_slice(&frame) {
            Ok(res) => res,
            Err(parse_err) => {
                return match serde_json::from_slice::<JsonRpcErrorResponse>(&frame) {
                    Ok(err_res) => (Err(err_res.error.into()), Some(conn_wrapper)),
                    Err(_) => (Err(parse_err.into()), Some(conn_wrapper)),
                };
            }
        };
        (Ok(res), Some(conn_wrapper))
    }

    pub(crate) async fn open_request_stream(
        &self,
        interface_name: &str,
        request: &JsonRpcRequest,
    ) -> Result<OwnedStream> {
        let req_bytes = serde_json::to_vec(request)?;
        check_frame_size(&request.method, &req_bytes)?;

        let conn_wrapper = self.connection().context("Not connected")?;
        let sent = self.send_request(&conn_wrapper, interface_name, &req_bytes, false).await?;
        match sent {
            Sent::Open(s) => Ok(s),
            Sent::Answered(f) => {
                if let Ok(err_res) = serde_json::from_slice::<JsonRpcErrorResponse>(&f) {
                    Err(err_res.error.into())
                } else {
                    Err(anyhow::anyhow!("the node answered before the subscribe request was sent"))
                }
            }
        }
    }
}
