//! Async I/O copy loops and bridge utilities
//!
//! Handles bidirectional copy tasks and framing adapters for bridged streams.

use super::RouteHandler;
use crate::preamble::{RoutePreamble, RouteTransport};
use anyhow::{Result, anyhow};
use hyper_util::rt::TokioIo;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, ReadBuf,
};
use tracing::debug;

/// A simple wrapper that combines an `AsyncRead` and `AsyncWrite` into a single type.
///
/// This is useful when you have split a stream into halves but need to pass them
/// as a single object to a library like `hyper`.
pub struct ReaderWriter<R, W> {
    pub reader: R,
    pub writer: W,
}

impl<R: AsyncRead + Unpin, W: Unpin> AsyncRead for ReaderWriter<R, W> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}

impl<R: Unpin, W: AsyncWrite + Unpin> AsyncWrite for ReaderWriter<R, W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.writer).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.writer).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.writer).poll_shutdown(cx)
    }
}

/// Reads a single line from the reader and parses it as a `RoutePreamble`.
pub async fn read_preamble<R>(reader: &mut BufReader<R>) -> Result<RoutePreamble>
where
    R: AsyncRead + Unpin,
{
    let mut raw_preamble = String::new();
    let read = reader.read_line(&mut raw_preamble).await?;
    if read == 0 {
        return Err(anyhow!("Stream closed before reading preamble"));
    }

    RoutePreamble::parse(&raw_preamble)
}

impl RouteHandler {
    /// The main entry point for handling an incoming stream.
    ///
    /// It first reads the `RoutePreamble` to determine the transport and protocol,
    /// then dispatches to the appropriate handler (HTTP or Binary).
    pub async fn handle_stream<S>(self, stream: S) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (read_half, write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let mut writer = write_half;

        // All streams now start with a preamble identifying transport and protocol.
        debug!("[Router] Reading preamble from incoming stream");
        let preamble = read_preamble(&mut reader).await?;
        debug!(
            "[Router] Preamble received: transport={:?} protocol={:?} interface='{}' service_id='{}' enc={:?}",
            preamble.transport,
            preamble.protocol,
            preamble.interface,
            preamble.service_id,
            preamble.enc
        );

        let resolved_route = self.resolve_route(preamble)?;
        debug!("[Router] Route resolved: endpoint={:?}", resolved_route.endpoint);
        let routing_plan = self.plan_route(&resolved_route);
        debug!(
            "[Router] Routing plan: delivery_mode={:?} execution={:?}",
            routing_plan.delivery_mode, routing_plan.execution
        );
        super::dispatch::log_route(&resolved_route, &routing_plan);

        use crate::routing::{DeliveryMode, RouteExecution};

        // Handle Passthrough delivery first, regardless of transport.
        // This is used for raw TCP proxying and wRPC passthrough.
        if routing_plan.delivery_mode == DeliveryMode::PassThrough {
            match &routing_plan.execution {
                RouteExecution::TcpPassthrough { host, port } => {
                    debug!("[Router] TcpPassthrough: connecting to {}:{}", host, port);
                    let mut target = tokio::net::TcpStream::connect(format!("{}:{}", host, port))
                        .await
                        .map_err(|e| {
                            anyhow!("Failed to connect to TCP target {host}:{port}: {e}")
                        })?;
                    debug!("[Router] TCP connection to {}:{} established", host, port);

                    if let (Some(enc), Some(pubkey_hex)) =
                        (&resolved_route.request.enc, &resolved_route.request.pubkey)
                        && enc == "ecdh-p256"
                    {
                        debug!("[Router] ECDH-P256 requested; performing in-line key exchange");
                        let client_pub_key_bytes = hex::decode(pubkey_hex)
                            .map_err(|e| anyhow!("Invalid hex pubkey: {e}"))?;

                        let client_pub_key = p256::EncodedPoint::from_bytes(&client_pub_key_bytes)
                            .map_err(|e| anyhow!("Invalid public key bytes: {e}"))?;

                        let public_key =
                            p256::PublicKey::from_sec1_bytes(client_pub_key.as_bytes())
                                .map_err(|e| anyhow!("Invalid public key point: {e}"))?;

                        let secret = p256::ecdh::EphemeralSecret::random(&mut rand::rngs::OsRng);
                        let server_pub_key = p256::EncodedPoint::from(secret.public_key());

                        let shared = secret.diffie_hellman(&public_key);
                        let shared_bytes = shared.raw_secret_bytes();

                        let mut key = [0u8; 32];
                        key.copy_from_slice(&shared_bytes[..32]);

                        use aes_gcm::{Aes256Gcm, KeyInit};
                        let cipher =
                            Arc::new(Aes256Gcm::new(aes_gcm::Key::<Aes256Gcm>::from_slice(&key)));

                        // Send our server public key and signature
                        let mut payload = Vec::with_capacity(130);
                        payload.extend_from_slice(server_pub_key.as_bytes());
                        payload.extend_from_slice(&client_pub_key_bytes);

                        let signature = self.inner.identity.sign(&payload);
                        let signature_bytes = signature.to_bytes();

                        debug!(
                            "[Router] Sending server public key ({} bytes) and signature ({} bytes)",
                            server_pub_key.as_bytes().len(),
                            signature_bytes.len()
                        );
                        writer.write_all(server_pub_key.as_bytes()).await?;
                        writer.write_all(&signature_bytes).await?;
                        debug!(
                            "[Router] ECDH key exchange complete; starting encrypted bidirectional pipe"
                        );

                        // Now, perform encrypted loop!
                        let (mut tcp_read, mut tcp_write) = target.into_split();

                        let cipher_recv = cipher.clone();
                        let mut recv_stream = reader;
                        let recv_fut = async move {
                            for _ in 0.. {
                                match read_decrypted_chunk(&mut recv_stream, &cipher_recv).await {
                                    Ok(Some(plaintext)) => {
                                        if plaintext.is_empty() {
                                            break;
                                        }
                                        if tcp_write.write_all(&plaintext).await.is_err() {
                                            break;
                                        }
                                    }
                                    Ok(None) => break,
                                    Err(_) => break,
                                }
                            }
                            let _ = tcp_write.shutdown().await;
                        };

                        let cipher_send = cipher;
                        let mut send_stream = writer;
                        let send_fut = async move {
                            let mut buf = vec![0u8; 16384];
                            loop {
                                match tcp_read.read(&mut buf).await {
                                    Ok(0) => {
                                        let _ = write_encrypted_chunk(
                                            &mut send_stream,
                                            &cipher_send,
                                            &[],
                                        )
                                        .await;
                                        break;
                                    }
                                    Ok(n) => {
                                        if write_encrypted_chunk(
                                            &mut send_stream,
                                            &cipher_send,
                                            &buf[..n],
                                        )
                                        .await
                                        .is_err()
                                        {
                                            break;
                                        }
                                    }
                                    Err(_) => break,
                                }
                            }
                            let _ = send_stream.shutdown().await;
                        };

                        tokio::select! {
                            _ = recv_fut => {}
                            _ = send_fut => {}
                        }

                        return Ok(());
                    }

                    let mut client = ReaderWriter { reader, writer };
                    tokio::io::copy_bidirectional(&mut client, &mut target).await.map_err(|e| {
                        anyhow!("Error in bidirectional copy for {host}:{port}: {e}")
                    })?;
                    return Ok(());
                }
                RouteExecution::WasmWrpcPassthrough { channel_id } => {
                    // wRPC passthrough is not yet implemented; log and continue.
                    debug!("Passthrough wRPC stream to Wasm channel: {}", channel_id);
                    tracing::warn!(
                        channel_id = %channel_id,
                        "wRPC passthrough not yet implemented; dropping stream"
                    );
                    return Ok(());
                }
                _ => {
                    return Err(anyhow!(
                        "Invalid execution plan for PassThrough delivery: {:?}",
                        routing_plan.execution
                    ));
                }
            }
        }

        match resolved_route.request.transport {
            RouteTransport::Http => {
                // Reunite reader + writer into a single I/O type for hyper.
                let io = TokioIo::new(ReaderWriter { reader, writer });
                return self.handle_http_stream(io, resolved_route.request).await;
            }
            RouteTransport::Binary => match &routing_plan.execution {
                RouteExecution::NativeJsonRpc { .. }
                | RouteExecution::ExecuteWasm { .. }
                | RouteExecution::Adapted { .. } => {
                    self.handle_json_rpc_loop(reader, &mut writer, &resolved_route, &routing_plan)
                        .await?;
                }
                RouteExecution::Unsupported => {
                    tracing::warn!(
                        protocol = %resolved_route.request.protocol,
                        interface = resolved_route.request.interface.as_str(),
                        service_id = resolved_route.request.service_id.as_str(),
                        "unsupported routing combination"
                    );
                }
                _ => {
                    return Err(anyhow!(
                        "Execution plan {:?} not supported for Binary transport",
                        routing_plan.execution
                    ));
                }
            },
            RouteTransport::Raw => {
                // We should have handled PassThrough above. If we're here with Raw transport
                // but not PassThrough, something is misconfigured.
                return Err(anyhow!(
                    "Raw transport requires PassThrough delivery, but plan has {:?}",
                    routing_plan.delivery_mode
                ));
            }
        }

        Ok(())
    }
}

async fn write_encrypted_chunk<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    cipher: &aes_gcm::Aes256Gcm,
    plaintext: &[u8],
) -> anyhow::Result<()> {
    use aes_gcm::aead::Aead;
    use rand::RngCore;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

    let payload_len = (12 + ciphertext.len()) as u16;
    writer.write_u16(payload_len).await?;
    writer.write_all(&nonce_bytes).await?;
    writer.write_all(&ciphertext).await?;
    Ok(())
}

async fn read_decrypted_chunk<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    cipher: &aes_gcm::Aes256Gcm,
) -> anyhow::Result<Option<Vec<u8>>> {
    use aes_gcm::aead::Aead;
    let payload_len = match reader.read_u16().await {
        Ok(len) => len,
        Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    if payload_len < 12 {
        return Err(anyhow::anyhow!("Invalid chunk length: {}", payload_len));
    }

    let mut payload = vec![0u8; payload_len as usize];
    reader.read_exact(&mut payload).await?;

    let nonce = aes_gcm::Nonce::from_slice(&payload[..12]);
    let ciphertext = &payload[12..];

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("Decryption failed: {}", e))?;

    Ok(Some(plaintext))
}
