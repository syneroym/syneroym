use std::io::Result;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};
use tracing::{debug, error};
use webrtc::data::data_channel::DataChannel as DetachedDataChannel;

/// A wrapper around WebRTC DetachedDataChannel that implements
/// tokio::io::AsyncRead and tokio::io::AsyncWrite.
///
/// Since DetachedDataChannel exposes an async API (not poll-based) and is message-oriented,
/// this wrapper spawns a background task to bridge the data between the channel and
/// a tokio::io::duplex stream.
pub struct WebRTCStream {
    inner: DuplexStream,
}

impl WebRTCStream {
    pub fn new(channel: Arc<DetachedDataChannel>) -> Self {
        let (local, remote) = tokio::io::duplex(65536); // 64KB buffer

        // Split the duplex stream into read and write halves
        let (mut remote_read, mut remote_write) = tokio::io::split(remote);
        let channel_read = channel.clone();
        let channel_write = channel.clone();

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
                            let data = bytes::Bytes::copy_from_slice(&buf_out[..n]);
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
            });

            // Wait for both to finish (or one to fail/close, depending on desired semantics)
            // Usually, if one side closes, we might want to tear down the other,
            // but keeping them independent allows half-open connections if supported.
            // For simplicity here, we just await both.
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
    ) -> Poll<Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for WebRTCStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
