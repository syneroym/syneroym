//! Iroh network client/server channel routing
//!
//! Adapts Iroh peer endpoints to the connection router, handling secure tunnel
//! connection requests.

use std::{
    fmt::{Debug, Formatter},
    pin::Pin,
    task::{Context as TaskContext, Poll},
};

use iroh::{
    Endpoint, RelayMap, RelayMode, RelayUrl, SecretKey,
    endpoint::{Connection, RecvStream, SendStream, presets::N0},
};
use syneroym_core::{config::RetryPolicy, retry::retry_with_backoff};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub struct IrohStream {
    send: SendStream,
    recv: RecvStream,
    conn: Option<iroh::endpoint::Connection>,
}

impl Debug for IrohStream {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IrohStream")
            .field("send", &"iroh::endpoint::SendStream")
            .field("recv", &"iroh::endpoint::RecvStream")
            .field("conn", &self.conn.is_some())
            .finish()
    }
}

impl IrohStream {
    #[must_use]
    pub fn new(send: SendStream, recv: RecvStream) -> Self {
        Self { send, recv, conn: None }
    }

    #[must_use]
    pub fn with_conn(mut self, conn: iroh::endpoint::Connection) -> Self {
        self.conn = Some(conn);
        self
    }
}

impl AsyncRead for IrohStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.recv).poll_read(cx, buf)
    }
}

impl AsyncWrite for IrohStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.send).poll_write(cx, buf).map_err(std::io::Error::other)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.send).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.send).poll_shutdown(cx)
    }
}

pub async fn build_iroh_endpoint(
    relay_url: Option<String>,
    secret_key: Option<SecretKey>,
    idle_timeout_secs: Option<u64>,
) -> anyhow::Result<Endpoint> {
    let mut builder = Endpoint::builder(N0);
    if let Some(url) = relay_url {
        match url.parse::<RelayUrl>() {
            Ok(relay_url) => {
                builder = Endpoint::empty_builder()
                    .relay_mode(RelayMode::Custom(RelayMap::from(relay_url)));
            }
            Err(e) => {
                tracing::warn!("Failed to parse relay URL '{}': {}, falling back to N0", url, e);
            }
        }
    }
    if let Some(sk) = secret_key {
        builder = builder.secret_key(sk);
    }

    if let Some(timeout) = idle_timeout_secs {
        let mut builder_cfg = iroh::endpoint::QuicTransportConfig::builder();
        builder_cfg =
            builder_cfg.max_idle_timeout(Some(std::time::Duration::from_secs(timeout).try_into()?));
        builder = builder.transport_config(builder_cfg.build());
    }

    let endpoint = builder.bind().await?;
    Ok(endpoint)
}

/// Connects to an Iroh endpoint with exponential backoff retries.
pub async fn connect_with_retry(
    endpoint: &Endpoint,
    node_addr: iroh::EndpointAddr,
    alpn: &[u8],
    retry_policy: &RetryPolicy,
) -> anyhow::Result<Connection> {
    let node_addr_clone = node_addr.clone();
    retry_with_backoff(retry_policy, || {
        let ep = endpoint.clone();
        let addr = node_addr_clone.clone();
        async move { ep.connect(addr, alpn).await }
    })
    .await
    .map_err(anyhow::Error::from)
}
