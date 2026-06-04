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
    endpoint::{RecvStream, SendStream, presets::N0},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub struct IrohStream {
    send: SendStream,
    recv: RecvStream,
}

impl Debug for IrohStream {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IrohStream")
            .field("send", &"iroh::endpoint::SendStream")
            .field("recv", &"iroh::endpoint::RecvStream")
            .finish()
    }
}

impl IrohStream {
    #[must_use]
    pub const fn new(send: SendStream, recv: RecvStream) -> Self {
        Self { send, recv }
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
) -> anyhow::Result<Endpoint> {
    let mut builder = Endpoint::builder(N0);
    if let Some(url) = relay_url
        && let Ok(relay_url) = url.parse::<RelayUrl>()
    {
        builder = builder.relay_mode(RelayMode::Custom(RelayMap::from(relay_url)));
    }
    if let Some(sk) = secret_key {
        builder = builder.secret_key(sk);
    }
    let endpoint = builder.bind().await?;
    Ok(endpoint)
}
