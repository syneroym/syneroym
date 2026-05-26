//! Stream encryption and decryption stages
//!
//! Encapsulates ECDH-P256 handshakes, signature verification, and AES-256-GCM chunk framing.

use crate::preamble::RoutePreamble;
use crate::routing::EncryptionStage;
use anyhow::{Result, anyhow};
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tracing::debug;

macro_rules! ready {
    ($e:expr $(,)?) => {
        match $e {
            std::task::Poll::Ready(t) => t,
            std::task::Poll::Pending => return std::task::Poll::Pending,
        }
    };
}

/// A simple wrapper that combines an `AsyncRead` and `AsyncWrite` into a single type.
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

/// A type-erased bidirectional stream, owned by the caller.
/// Used after the encryption stage to allow downstream stages to be
/// generic over whether the stream is encrypted or plaintext.
pub type OwnedStream = ReaderWriter<
    Box<dyn tokio::io::AsyncRead + Unpin + Send>,
    Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
>;

enum ReadState {
    ReadLen { buf: [u8; 2], bytes_read: usize },
    ReadPayload { len: u16, buf: Vec<u8>, bytes_read: usize },
    Decrypted { buf: Vec<u8>, pos: usize },
    Eof,
}

struct EncryptedReader<R> {
    reader: R,
    cipher: aes_gcm::Aes256Gcm,
    state: ReadState,
}

impl<R: AsyncRead + Unpin> AsyncRead for EncryptedReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        loop {
            match &mut this.state {
                ReadState::Decrypted { buf: decrypted_buf, pos } => {
                    if *pos < decrypted_buf.len() {
                        let to_read = std::cmp::min(decrypted_buf.len() - *pos, buf.remaining());
                        buf.put_slice(&decrypted_buf[*pos..*pos + to_read]);
                        *pos += to_read;
                        return Poll::Ready(Ok(()));
                    } else {
                        this.state = ReadState::ReadLen { buf: [0; 2], bytes_read: 0 };
                    }
                }
                ReadState::ReadLen { buf: len_buf, bytes_read } => {
                    while *bytes_read < 2 {
                        let mut temp_buf = ReadBuf::new(&mut len_buf[*bytes_read..]);
                        ready!(Pin::new(&mut this.reader).poll_read(cx, &mut temp_buf))?;
                        let n = temp_buf.filled().len();
                        if n == 0 {
                            if *bytes_read == 0 {
                                this.state = ReadState::Eof;
                                return Poll::Ready(Ok(()));
                            } else {
                                return Poll::Ready(Err(std::io::Error::new(
                                    std::io::ErrorKind::UnexpectedEof,
                                    "unexpected EOF reading chunk length",
                                )));
                            }
                        }
                        *bytes_read += n;
                    }
                    let payload_len = u16::from_be_bytes(*len_buf);
                    if payload_len < 12 {
                        return Poll::Ready(Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "invalid chunk length",
                        )));
                    }
                    this.state = ReadState::ReadPayload {
                        len: payload_len,
                        buf: vec![0; payload_len as usize],
                        bytes_read: 0,
                    };
                }
                ReadState::ReadPayload { len, buf: payload_buf, bytes_read } => {
                    let target = *len as usize;
                    while *bytes_read < target {
                        let mut temp_buf = ReadBuf::new(&mut payload_buf[*bytes_read..]);
                        ready!(Pin::new(&mut this.reader).poll_read(cx, &mut temp_buf))?;
                        let n = temp_buf.filled().len();
                        if n == 0 {
                            return Poll::Ready(Err(std::io::Error::new(
                                std::io::ErrorKind::UnexpectedEof,
                                "unexpected EOF reading chunk payload",
                            )));
                        }
                        *bytes_read += n;
                    }
                    // Decrypt payload
                    let nonce = aes_gcm::Nonce::from_slice(&payload_buf[..12]);
                    let ciphertext = &payload_buf[12..];
                    use aes_gcm::aead::Aead;
                    let plaintext = this.cipher.decrypt(nonce, ciphertext).map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Decryption failed: {}", e),
                        )
                    })?;
                    this.state = ReadState::Decrypted { buf: plaintext, pos: 0 };
                }
                ReadState::Eof => {
                    return Poll::Ready(Ok(()));
                }
            }
        }
    }
}

struct PendingWrite {
    buf: Vec<u8>,
    pos: usize,
}

struct EncryptedWriter<W> {
    writer: W,
    cipher: aes_gcm::Aes256Gcm,
    write_buf: Vec<u8>,
    pending_write: Option<PendingWrite>,
}

impl<W: AsyncWrite + Unpin> EncryptedWriter<W> {
    fn encrypt_and_start_write(&mut self) -> std::io::Result<()> {
        if self.write_buf.is_empty() {
            return Ok(());
        }
        use aes_gcm::aead::Aead;
        use rand::RngCore;
        let mut nonce_bytes = [0u8; 12];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);

        let ciphertext = self
            .cipher
            .encrypt(nonce, self.write_buf.as_slice())
            .map_err(|e| std::io::Error::other(format!("Encryption failed: {}", e)))?;

        let payload_len = (12 + ciphertext.len()) as u16;
        let mut pending_buf = Vec::with_capacity(2 + 12 + ciphertext.len());
        pending_buf.extend_from_slice(&payload_len.to_be_bytes());
        pending_buf.extend_from_slice(&nonce_bytes);
        pending_buf.extend_from_slice(&ciphertext);

        self.pending_write = Some(PendingWrite { buf: pending_buf, pos: 0 });
        self.write_buf.clear();
        Ok(())
    }

    fn poll_flush_pending(&mut self, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        if let Some(pending) = &mut self.pending_write {
            while pending.pos < pending.buf.len() {
                let n =
                    ready!(Pin::new(&mut self.writer).poll_write(cx, &pending.buf[pending.pos..]))?;
                if n == 0 {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "failed to write encrypted chunk to underlying stream",
                    )));
                }
                pending.pos += n;
            }
            self.pending_write = None;
        }
        Poll::Ready(Ok(()))
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for EncryptedWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = &mut *self;
        // First, flush any pending encrypted write
        ready!(this.poll_flush_pending(cx))?;

        // Buffer the incoming plaintext
        let space = 8192 - this.write_buf.len();
        let to_write = std::cmp::min(space, buf.len());
        this.write_buf.extend_from_slice(&buf[..to_write]);

        // If the buffer is full, encrypt and start writing it
        if this.write_buf.len() >= 8192 {
            this.encrypt_and_start_write()?;
        }

        Poll::Ready(Ok(to_write))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        // If we have buffered plaintext, encrypt and start writing it
        if !this.write_buf.is_empty() {
            this.encrypt_and_start_write()?;
        }
        // Flush any pending encrypted write
        ready!(this.poll_flush_pending(cx))?;
        // Finally, flush the underlying writer
        Pin::new(&mut this.writer).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        // First, flush all buffered plaintext and pending writes
        ready!(self.as_mut().poll_flush(cx))?;
        // Then shutdown the underlying writer
        Pin::new(&mut self.get_mut().writer).poll_shutdown(cx)
    }
}

/// Applies the encryption stage to the stream, returning an OwnedStream
/// that downstream transport/service stages can use without knowing
/// whether the wire is encrypted.
pub async fn apply_encryption_stage<R, W>(
    reader: R,
    mut writer: W,
    stage: &EncryptionStage,
    preamble: &RoutePreamble,
    identity: &syneroym_identity::Identity,
) -> Result<OwnedStream>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    match stage {
        EncryptionStage::None => Ok(ReaderWriter {
            reader: Box::new(reader) as Box<dyn AsyncRead + Unpin + Send>,
            writer: Box::new(writer) as Box<dyn AsyncWrite + Unpin + Send>,
        }),
        EncryptionStage::TerminateEcdhP256 => {
            debug!("[Router] TerminateEcdhP256 encryption stage: performing handshake");

            let pubkey_hex = preamble.pubkey.as_deref().ok_or_else(|| {
                anyhow!("Missing 'pubkey' query parameter required for ecdh-p256 encryption")
            })?;

            let client_pub_key_bytes = hex::decode(pubkey_hex)
                .map_err(|e| anyhow!("Invalid hex pubkey in preamble: {e}"))?;

            let client_pub_key = p256::EncodedPoint::from_bytes(&client_pub_key_bytes)
                .map_err(|e| anyhow!("Invalid public key bytes: {e}"))?;

            let public_key = p256::PublicKey::from_sec1_bytes(client_pub_key.as_bytes())
                .map_err(|e| anyhow!("Invalid public key point: {e}"))?;

            let secret = p256::ecdh::EphemeralSecret::random(&mut aes_gcm::aead::OsRng);
            let server_pub_key = p256::EncodedPoint::from(secret.public_key());

            let shared = secret.diffie_hellman(&public_key);
            let shared_bytes = shared.raw_secret_bytes();

            let mut key = [0u8; 32];
            key.copy_from_slice(&shared_bytes[..32]);

            use aes_gcm::KeyInit;
            let cipher =
                aes_gcm::Aes256Gcm::new(aes_gcm::Key::<aes_gcm::Aes256Gcm>::from_slice(&key));

            // Send our server public key and signature
            let mut payload = Vec::with_capacity(130);
            payload.extend_from_slice(server_pub_key.as_bytes());
            payload.extend_from_slice(&client_pub_key_bytes);

            let signature = identity.sign(&payload);
            let signature_bytes = signature.to_bytes();

            debug!(
                "[Router] Sending server public key ({} bytes) and signature ({} bytes)",
                server_pub_key.as_bytes().len(),
                signature_bytes.len()
            );

            writer.write_all(server_pub_key.as_bytes()).await?;
            writer.write_all(&signature_bytes).await?;
            writer.flush().await?;

            debug!("[Router] ECDH key exchange complete; starting encrypted bidirectional pipe");

            let encrypted_reader = EncryptedReader {
                reader,
                cipher: cipher.clone(),
                state: ReadState::ReadLen { buf: [0; 2], bytes_read: 0 },
            };

            let encrypted_writer = EncryptedWriter {
                writer,
                cipher,
                write_buf: Vec::with_capacity(8192),
                pending_write: None,
            };

            Ok(ReaderWriter {
                reader: Box::new(encrypted_reader) as Box<dyn AsyncRead + Unpin + Send>,
                writer: Box::new(encrypted_writer) as Box<dyn AsyncWrite + Unpin + Send>,
            })
        }
    }
}
