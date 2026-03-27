use anyhow::Result;
use iroh::endpoint::{Connection, RelayMode, presets};
use iroh::protocol::{AcceptError, ProtocolHandler, Router as IrohRouter};
use iroh::{RelayUrl, SecretKey};
use iroh_relay::RelayMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use syneroym_core::config::IrohRelayConfig;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tracing::{debug, error, info};

use crate::ConnectionRouter;

pub const SYNEROYM_ALPN: &[u8] = b"syneroym/0.1";

pub async fn init(
    config: &IrohRelayConfig,
    secret_key: SecretKey,
    router: Arc<ConnectionRouter>,
) -> Result<Option<IrohRouter>> {
    debug!("Initializing Iroh communication...");

    // Bind endpoint
    let mut ep_bldr = iroh::Endpoint::builder(presets::N0);
    // If a relay URL is provided in the config, use it. Otherwise, the default from presets::N0 will be used.
    if let Ok(relay_url) = config.relay_url.parse::<RelayUrl>() {
        ep_bldr = iroh::Endpoint::empty_builder()
            .relay_mode(RelayMode::Custom(RelayMap::from(relay_url)));
    }

    let ep_bldr = ep_bldr.secret_key(secret_key);
    let ep = ep_bldr.bind().await?;

    let iroh_router = IrohRouter::builder(ep)
        .accept(SYNEROYM_ALPN, Arc::new(ConnectionHandler { router }))
        .spawn();

    info!("Iroh listening on ALPN: {:?}", std::str::from_utf8(SYNEROYM_ALPN).unwrap());

    Ok(Some(iroh_router))
}

#[derive(Debug, Clone)]
struct ConnectionHandler {
    router: Arc<ConnectionRouter>,
}

impl ProtocolHandler for ConnectionHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let endpoint_id = connection.remote_id();
        debug!("accepted connection from {endpoint_id}");

        // We expect the connecting peer to open a single bi-directional stream.
        let (send, recv) = connection.accept_bi().await?;

        let iroh_stream = IrohStream::new(send, recv);
        if let Err(e) = self.router.handle_stream(iroh_stream).await {
            error!("Error handling Iroh stream: {}", e);
        }

        // Wait until the remote closes the connection, which it does once it
        // received the response.
        connection.closed().await;

        Ok(())
    }
}

pub struct IrohStream {
    send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
}

impl IrohStream {
    pub fn new(send: iroh::endpoint::SendStream, recv: iroh::endpoint::RecvStream) -> Self {
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
