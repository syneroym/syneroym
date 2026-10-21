use std::{
    cmp,
    io::{self, ErrorKind},
    pin::Pin,
    task::{Context as TaskContext, Poll, ready},
};

use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, KeyInit},
};
use rand::RngCore;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// A simple wrapper that combines an `AsyncRead` and `AsyncWrite` into a single
/// type.
pub struct ReaderWriter<R, W> {
    pub reader: R,
    pub writer: W,
}

impl<R, W> std::fmt::Debug for ReaderWriter<R, W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReaderWriter").finish()
    }
}

impl<R: AsyncRead + Unpin, W: Unpin> AsyncRead for ReaderWriter<R, W> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}

impl<R: Unpin, W: AsyncWrite + Unpin> AsyncWrite for ReaderWriter<R, W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.writer).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.writer).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.writer).poll_shutdown(cx)
    }
}

/// A type-erased bidirectional stream, owned by the caller.
/// Used after the encryption stage to allow downstream stages to be
/// generic over whether the stream is encrypted or plaintext.
pub type OwnedStream =
    ReaderWriter<Box<dyn AsyncRead + Unpin + Send>, Box<dyn AsyncWrite + Unpin + Send>>;

enum ReadState {
    ReadLen { buf: [u8; 2], bytes_read: usize },
    ReadPayload { len: u16, buf: Vec<u8>, bytes_read: usize },
    Decrypted { buf: Vec<u8>, pos: usize },
    Eof,
}

struct EncryptedReader<R> {
    reader: R,
    cipher: Aes256Gcm,
    state: ReadState,
}

impl<R: AsyncRead + Unpin> AsyncRead for EncryptedReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        loop {
            match &mut this.state {
                ReadState::Decrypted { buf: decrypted_buf, pos } => {
                    if *pos < decrypted_buf.len() {
                        let to_read = cmp::min(decrypted_buf.len() - *pos, buf.remaining());
                        buf.put_slice(&decrypted_buf[*pos..*pos + to_read]);
                        *pos += to_read;
                        return Poll::Ready(Ok(()));
                    }
                    this.state = ReadState::ReadLen { buf: [0; 2], bytes_read: 0 };
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
                            }
                            return Poll::Ready(Err(io::Error::new(
                                ErrorKind::UnexpectedEof,
                                "unexpected EOF reading chunk length",
                            )));
                        }
                        *bytes_read += n;
                    }
                    let payload_len = u16::from_be_bytes(*len_buf);
                    if payload_len < 12 {
                        return Poll::Ready(Err(io::Error::new(
                            ErrorKind::InvalidData,
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
                            return Poll::Ready(Err(io::Error::new(
                                ErrorKind::UnexpectedEof,
                                "unexpected EOF reading chunk payload",
                            )));
                        }
                        *bytes_read += n;
                    }
                    // Decrypt payload
                    let nonce = Nonce::from_slice(&payload_buf[..12]);
                    let ciphertext = &payload_buf[12..];
                    let plaintext = this.cipher.decrypt(nonce, ciphertext).map_err(|e| {
                        io::Error::new(ErrorKind::InvalidData, format!("Decryption failed: {e}"))
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
    cipher: Aes256Gcm,
    write_buf: Vec<u8>,
    pending_write: Option<PendingWrite>,
}

impl<W: AsyncWrite + Unpin> EncryptedWriter<W> {
    fn encrypt_and_start_write(&mut self) -> io::Result<()> {
        if self.write_buf.is_empty() {
            return Ok(());
        }
        let mut nonce_bytes = [0u8; 12];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = self
            .cipher
            .encrypt(nonce, self.write_buf.as_slice())
            .map_err(|e| io::Error::other(format!("Encryption failed: {e}")))?;

        let payload_len = (12 + ciphertext.len()) as u16;
        let mut pending_buf = Vec::with_capacity(2 + 12 + ciphertext.len());
        pending_buf.extend_from_slice(&payload_len.to_be_bytes());
        pending_buf.extend_from_slice(&nonce_bytes);
        pending_buf.extend_from_slice(&ciphertext);

        self.pending_write = Some(PendingWrite { buf: pending_buf, pos: 0 });
        self.write_buf.clear();
        Ok(())
    }

    fn poll_flush_pending(&mut self, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        if let Some(pending) = &mut self.pending_write {
            while pending.pos < pending.buf.len() {
                let n =
                    ready!(Pin::new(&mut self.writer).poll_write(cx, &pending.buf[pending.pos..]))?;
                if n == 0 {
                    return Poll::Ready(Err(io::Error::new(
                        ErrorKind::WriteZero,
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
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        ready!(this.poll_flush_pending(cx))?;

        let space = 8192 - this.write_buf.len();
        let to_write = cmp::min(space, buf.len());
        this.write_buf.extend_from_slice(&buf[..to_write]);

        if this.write_buf.len() >= 8192 {
            this.encrypt_and_start_write()?;
        }

        Poll::Ready(Ok(to_write))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if !this.write_buf.is_empty() {
            this.encrypt_and_start_write()?;
        }
        ready!(this.poll_flush_pending(cx))?;
        Pin::new(&mut this.writer).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        ready!(self.as_mut().poll_flush(cx))?;
        Pin::new(&mut self.get_mut().writer).poll_shutdown(cx)
    }
}

/// Wraps a stream in the chunk cipher: writes are sealed with `send_key`,
/// reads opened with `recv_key`.
pub(crate) fn wrap<R, W>(
    reader: R,
    writer: W,
    send_key: &[u8; 32],
    recv_key: &[u8; 32],
) -> OwnedStream
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let reader = EncryptedReader {
        reader,
        cipher: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(recv_key)),
        state: ReadState::ReadLen { buf: [0; 2], bytes_read: 0 },
    };
    let writer = EncryptedWriter {
        writer,
        cipher: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(send_key)),
        write_buf: Vec::with_capacity(8192),
        pending_write: None,
    };
    ReaderWriter { reader: Box::new(reader), writer: Box::new(writer) }
}
