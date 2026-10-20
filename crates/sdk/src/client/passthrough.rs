use anyhow::{Context, Result};
use iroh::endpoint::{RecvStream, SendStream, WriteError};
use syneroym_identity::{DelegationCertificate, Identity};
use syneroym_router::{REFUSAL_HEADER, RoutePreamble, RouteProtocol, RouteTransport};
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
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
            TransportConnection::Iroh { conn, .. } => {
                let (mut send, mut recv) = conn.open_bi().await?;

                let mut preamble = RoutePreamble::new(
                    RouteTransport::Http,
                    RouteProtocol::JsonRpc,
                    service_id.to_string(),
                    interface_name.to_string(),
                );
                preamble.delegation = delegation.cloned();
                preamble.sign(identity, now_secs());

                let mut write_err = None;
                if let Err(e) = send.write_all(preamble.to_preamble_line().as_bytes()).await {
                    write_err = Some(e);
                } else if let Err(e) = send.write_all(initial_bytes).await {
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
    recv: &mut RecvStream,
    send: &mut SendStream,
    tcp_stream: &mut TcpStream,
    first: &mut [u8],
    write_err: &mut Option<WriteError>,
    tcp_open: &mut bool,
    client_sent_more: &mut bool,
) -> Result<usize> {
    let mut buf = vec![0u8; 4096];
    loop {
        if let Some(err) = write_err.take() {
            match time::timeout(REFUSAL_READ_GRACE, recv.read(first)).await {
                Ok(Ok(Some(n))) => return Ok(n),
                Ok(Ok(None)) => return Err(err.into()),
                Ok(Err(e)) => return Err(anyhow::anyhow!(e)),
                Err(_) => return Err(err.into()),
            }
        }

        if !*tcp_open {
            let r = recv.read(first).await?;
            return Ok(r.unwrap_or(0));
        }

        tokio::select! {
            r = recv.read(first) => {
                let n = r?.unwrap_or(0);
                return Ok(n);
            }
            r = tcp_stream.read(&mut buf), if *tcp_open => {
                match r {
                    Ok(0) => {
                        let _ = send.finish();
                        *tcp_open = false;
                    }
                    Ok(n) => {
                        if let Err(e) = send.write_all(&buf[..n]).await {
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
