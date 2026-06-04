//! WebRTC network client/server channel routing
//!
//! Adapts active WebRTC data channels to the connection router, mapping SDP/ICE candidates.

use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use bytes::Bytes;
use tokio::{
    io,
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf},
};
use tracing::{debug, error};
use webrtc::data::data_channel::DataChannel as DetachedDataChannel;

/// A wrapper around WebRTC `DetachedDataChannel` that implements
/// `tokio::io::AsyncRead` and `tokio::io::AsyncWrite`.
///
/// Since `DetachedDataChannel` exposes an async API (not poll-based) and is message-oriented,
/// this wrapper spawns a background task to bridge the data between the channel and
/// a `tokio::io::duplex` stream.
#[derive(Debug)]
pub struct WebRTCStream {
    inner: DuplexStream,
}

impl WebRTCStream {
    #[must_use]
    pub fn new(channel: Arc<DetachedDataChannel>) -> Self {
        let (local, remote) = io::duplex(65536); // 64KB buffer

        // Split the duplex stream into read and write halves
        let (mut remote_read, mut remote_write) = io::split(remote);
        let channel_read = channel.clone();
        let channel_write = channel;

        tokio::spawn(async move {
            // Task 1: Read from WebRTC -> Write to Duplex
            let inbound = tokio::spawn(async move {
                let mut buf_in = vec![0u8; 8192];
                loop {
                    match channel_read.read(&mut buf_in).await {
                        Ok(0) => break, // EOF
                        Ok(n) => {
                            if let Err(e) = remote_write.write_all(&buf_in[..n]).await {
                                debug!("WebRTCStream bridge: failed to write to duplex: {}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            error!("WebRTCStream bridge: WebRTC read error: {}", e);
                            break;
                        }
                    }
                }
                // Close the write side of the duplex to signal EOF to the user
                let _ = remote_write.shutdown().await;
            });

            // Task 2: Read from Duplex -> Write to WebRTC
            let outbound = tokio::spawn(async move {
                let mut buf_out = vec![0u8; 8192];
                loop {
                    match remote_read.read(&mut buf_out).await {
                        Ok(0) => break, // EOF
                        Ok(n) => {
                            let data = Bytes::copy_from_slice(&buf_out[..n]);
                            if let Err(e) = channel_write.write(&data).await {
                                error!("WebRTCStream bridge: WebRTC write error: {}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            debug!("WebRTCStream bridge: duplex read error: {}", e);
                            break;
                        }
                    }
                }
                // Explicitly close the DataChannel so the browser-side dc.onclose fires.
                // Without this, the TransformStream body is never closed and response.text() hangs.
                if let Err(e) = channel_write.close().await {
                    debug!("WebRTCStream bridge: DataChannel close error: {}", e);
                }
            });

            let _ = tokio::join!(inbound, outbound);
            debug!("WebRTCStream bridge tasks finished");
        });

        Self { inner: local }
    }
}

impl AsyncRead for WebRTCStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for WebRTCStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
