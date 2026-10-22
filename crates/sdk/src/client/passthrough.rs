use anyhow::{Context, Result};
use syneroym_identity::{DelegationCertificate, Identity};
use syneroym_router::{
    Opened, REFUSAL_HEADER, ReaderWriter, Refusal, RoutePreamble, RouteProtocol, RouteTransport,
    e2e_channel::{self, E2eError, OwnedStream},
};
use tokio::{
    io::{self, AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::{self, Duration},
};

use super::{SyneroymClient, TransportConnection, now_secs};

const REFUSAL_READ_GRACE: Duration = Duration::from_secs(2);

/// What happened to a passed-through HTTP request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassthroughOutcome {
    /// The host answered (any HTTP status); bytes were copied both ways.
    Done,
    /// The node refused with `not_hosted_here`. `replayable` is true when
    /// nothing was sent to the client and no client bytes beyond
    /// `initial_bytes` were consumed, so the caller may send the same
    /// request to the new host.
    NotHostedHere { replayable: bool },
    /// The node that answered the handshake is not the host the record
    /// names. Nothing was sent, so the caller may send the same request
    /// after it looks the service up again.
    HostMismatch,
}

/// Most bytes a refusal head may take before its blank line.
const MAX_REFUSAL_HEAD: usize = 4096;

/// The refusal an HTTP head names: status 4xx or 5xx and a known
/// `REFUSAL_HEADER` reason. `None` for anything else.
fn authorized_http_refusal(head: &[u8]) -> Option<Refusal> {
    if !(head.starts_with(b"HTTP/1.1 4") || head.starts_with(b"HTTP/1.1 5")) {
        return None;
    }
    String::from_utf8_lossy(head).lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        k.trim().eq_ignore_ascii_case(REFUSAL_HEADER).then(|| Refusal::from_reason(v.trim()))?
    })
}

/// Reads up to the blank line that ends the HTTP head, at most
/// `MAX_REFUSAL_HEAD` bytes.
async fn read_refusal_head(recv: &mut (impl AsyncRead + Unpin)) -> Result<Vec<u8>> {
    let mut head = Vec::new();
    let mut buf = [0u8; 512];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        anyhow::ensure!(head.len() < MAX_REFUSAL_HEAD, "refusal head is too long");
        let n = recv.read(&mut buf).await?;
        anyhow::ensure!(n > 0, "refused stream closed before the refusal head ended");
        head.extend_from_slice(&buf[..n]);
    }
    Ok(head)
}

pub(crate) fn starts_with_not_hosted_refusal(chunk: &[u8]) -> bool {
    if !chunk.starts_with(b"HTTP/1.1 421 ") {
        return false;
    }
    let header_end = chunk.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(chunk.len());
    let header_bytes = &chunk[..header_end];
    let header_str = String::from_utf8_lossy(header_bytes);
    for line in header_str.lines() {
        if let Some((k, v)) = line.split_once(':')
            && k.trim().eq_ignore_ascii_case(REFUSAL_HEADER)
            && v.trim() == "not_hosted_here"
        {
            return true;
        }
    }
    false
}

/// Handles a stream the node refused before the handshake. The client
/// gets the router's own copy of the refusal (`Refusal::http_response`),
/// never the received bytes: on an indirect path they passed a forwarder
/// that could have added any body.
async fn handle_refused_passthrough(
    s: OwnedStream,
    tcp_stream: &mut TcpStream,
) -> Result<PassthroughOutcome> {
    let ReaderWriter { reader: mut recv, writer: mut send } = s;
    let _ = send.shutdown().await;

    let head = time::timeout(e2e_channel::HANDSHAKE_TIMEOUT, read_refusal_head(&mut recv))
        .await
        .map_err(|_| anyhow::anyhow!("timeout waiting for refusal response"))??;
    let refusal = authorized_http_refusal(&head)
        .ok_or_else(|| anyhow::anyhow!("refusal response is not an authorized HTTP refusal"))?;
    if refusal == Refusal::NotHostedHere {
        return Ok(PassthroughOutcome::NotHostedHere { replayable: true });
    }

    tcp_stream.write_all(&refusal.http_response()).await?;
    let _ = tcp_stream.shutdown().await;
    Ok(PassthroughOutcome::Done)
}

impl SyneroymClient {
    pub async fn passthrough(
        &self,
        interface_name: &str,
        initial_bytes: &[u8],
        tcp_stream: &mut TcpStream,
        delegation: Option<&DelegationCertificate>,
    ) -> Result<PassthroughOutcome> {
        let conn_wrapper = self.connection().context("Not connected")?;
        Self::passthrough_with_conn(
            conn_wrapper,
            &self.service_id,
            interface_name,
            initial_bytes,
            tcp_stream,
            &self.identity,
            delegation,
        )
        .await
    }

    pub async fn passthrough_with_conn(
        conn_wrapper: TransportConnection,
        service_id: &str,
        interface_name: &str,
        initial_bytes: &[u8],
        tcp_stream: &mut TcpStream,
        identity: &Identity,
        delegation: Option<&DelegationCertificate>,
    ) -> Result<PassthroughOutcome> {
        match conn_wrapper {
            TransportConnection::Iroh { conn, host_key, .. } => {
                let path =
                    e2e_channel::path(conn.remote_id().as_bytes(), host_key.as_ref(), service_id)?;
                let (send, recv) = conn.open_bi().await?;

                let mut preamble = RoutePreamble::new(
                    RouteTransport::Http,
                    RouteProtocol::JsonRpc,
                    service_id.to_string(),
                    interface_name.to_string(),
                );
                preamble.delegation = delegation.cloned();
                preamble.sign(identity, now_secs());

                let stream = match e2e_channel::open_stream(recv, send, preamble, &path).await {
                    Ok(Opened::Ready(s)) => s,
                    Ok(Opened::Refused(s)) => {
                        return handle_refused_passthrough(s, tcp_stream).await;
                    }
                    Err(E2eError::BadHostSignature { .. }) => {
                        return Ok(PassthroughOutcome::HostMismatch);
                    }
                    Err(e) => return Err(e.into()),
                };

                let ReaderWriter { reader: mut recv, writer: mut send } = stream;
                let mut write_err = None;
                let wrote = async {
                    send.write_all(initial_bytes).await?;
                    send.flush().await
                }
                .await;
                if let Err(e) = wrote {
                    write_err = Some(e);
                }

                let mut client_sent_more = false;
                let mut tcp_open = write_err.is_none();
                let mut first = vec![0u8; 4096];

                let n = read_first_response(
                    &mut recv,
                    &mut send,
                    tcp_stream,
                    &mut first,
                    &mut write_err,
                    &mut tcp_open,
                    &mut client_sent_more,
                )
                .await?;

                if n == 0 {
                    if let Some(err) = write_err {
                        return Err(err.into());
                    }
                    return Ok(PassthroughOutcome::Done);
                }

                if starts_with_not_hosted_refusal(&first[..n]) {
                    if !client_sent_more {
                        return Ok(PassthroughOutcome::NotHostedHere { replayable: true });
                    }
                    tcp_stream.write_all(&first[..n]).await?;
                    return Ok(PassthroughOutcome::NotHostedHere { replayable: false });
                }

                tcp_stream.write_all(&first[..n]).await?;
                if !tcp_open {
                    let (_, mut tcp_w) = tcp_stream.split();
                    let _ = io::copy(&mut recv, &mut tcp_w).await;
                    let _ = tcp_w.shutdown().await;
                } else {
                    let mut joined_iroh = io::join(recv, send);
                    let _ = io::copy_bidirectional(tcp_stream, &mut joined_iroh).await;
                }
                Ok(PassthroughOutcome::Done)
            }
        }
    }
}

async fn read_first_response(
    recv: &mut (impl io::AsyncRead + Unpin),
    send: &mut (impl io::AsyncWrite + Unpin),
    tcp_stream: &mut TcpStream,
    first: &mut [u8],
    write_err: &mut Option<io::Error>,
    tcp_open: &mut bool,
    client_sent_more: &mut bool,
) -> Result<usize> {
    let mut buf = vec![0u8; 4096];
    loop {
        if let Some(err) = write_err.take() {
            match time::timeout(REFUSAL_READ_GRACE, recv.read(first)).await {
                Ok(Ok(n)) if n > 0 => return Ok(n),
                Ok(Ok(_)) => return Err(err.into()),
                Ok(Err(e)) => return Err(anyhow::anyhow!(e)),
                Err(_) => return Err(err.into()),
            }
        }

        if !*tcp_open {
            let n = recv.read(first).await?;
            return Ok(n);
        }

        tokio::select! {
            r = recv.read(first) => {
                let n = r?;
                return Ok(n);
            }
            r = tcp_stream.read(&mut buf), if *tcp_open => {
                match r {
                    Ok(0) => {
                        let _ = send.shutdown().await;
                        *tcp_open = false;
                    }
                    Ok(n) => {
                        let wrote = async {
                            send.write_all(&buf[..n]).await?;
                            send.flush().await
                        }
                        .await;
                        if let Err(e) = wrote {
                            *write_err = Some(e);
                            *tcp_open = false;
                        } else {
                            *client_sent_more = true;
                        }
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
