//! Length-prefixed and chunked frame encoders/decoders
//!
//! Implements framing protocol layers over async buffers, supporting
//! reliable message boundaries in network streams.

use std::io::ErrorKind;

use anyhow::{Result, anyhow};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Upper bound on a frame's declared length, checked before allocating a
/// buffer for it. `read_frame` is reachable pre-authentication (e.g. the
/// M3B Slice 6B stream preamble's initial payload, read before any capacity
/// check or WASM instantiation), so an attacker-controlled `u32` length
/// prefix must never drive an unbounded allocation. Framed payloads here
/// are always small control-plane/RPC messages or stream-open metadata --
/// bulk data instead moves as raw unframed bytes through
/// `syneroym_chunk_transfer` in fixed-size chunks -- so 16 MiB is generous
/// headroom, not a tight fit.
pub const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

/// Writes a length-prefixed frame to the writer.
/// The frame is prefixed with its length as a `u32` in big-endian format.
pub async fn write_frame<W>(writer: &mut W, frame: &[u8]) -> Result<()>
where
    W: AsyncWrite + Unpin + Send,
{
    let len = frame.len() as u32;
    writer.write_u32(len).await?;
    writer.write_all(frame).await?;
    Ok(())
}

/// Reads a length-prefixed frame from the reader.
/// The frame is expected to be prefixed with its length as a `u32` in
/// big-endian format.
pub async fn read_frame<R>(reader: &mut R) -> Result<Vec<u8>>
where
    R: AsyncRead + Unpin + Send,
{
    match reader.read_u32().await {
        Ok(len) if len > MAX_FRAME_SIZE => {
            Err(anyhow!("frame length {len} exceeds MAX_FRAME_SIZE ({MAX_FRAME_SIZE})"))
        }
        Ok(len) if len > 0 => {
            let mut frame = vec![0; len as usize];
            reader.read_exact(&mut frame).await?;
            Ok(frame)
        }
        Ok(_) => Ok(Vec::new()),
        Err(e) if e.kind() == ErrorKind::UnexpectedEof => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[tokio::test]
    async fn test_roundtrip() {
        let payload = b"hello, world!";
        let mut buf = Vec::new();
        write_frame(&mut buf, payload).await.unwrap();
        let mut cursor = Cursor::new(buf);
        let out = read_frame(&mut cursor).await.unwrap();
        assert_eq!(out, payload);
    }

    #[tokio::test]
    async fn test_empty_payload_writes_zero_len() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"").await.unwrap();
        // A zero-length u32 prefix is written, read_frame should return empty vec
        let mut cursor = Cursor::new(buf);
        let out = read_frame(&mut cursor).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn test_eof_returns_empty() {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        let out = read_frame(&mut cursor).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn test_multiple_frames() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"first").await.unwrap();
        write_frame(&mut buf, b"second").await.unwrap();
        let mut cursor = Cursor::new(buf);
        assert_eq!(read_frame(&mut cursor).await.unwrap(), b"first");
        assert_eq!(read_frame(&mut cursor).await.unwrap(), b"second");
    }

    #[tokio::test]
    async fn test_oversized_length_prefix_rejected_without_allocating() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(MAX_FRAME_SIZE + 1).to_be_bytes());
        let mut cursor = Cursor::new(buf);
        let err = read_frame(&mut cursor).await.unwrap_err();
        assert!(err.to_string().contains("exceeds MAX_FRAME_SIZE"));
    }

    #[tokio::test]
    async fn test_max_frame_size_boundary_accepted() {
        let payload = vec![0u8; MAX_FRAME_SIZE as usize];
        let mut buf = Vec::new();
        write_frame(&mut buf, &payload).await.unwrap();
        let mut cursor = Cursor::new(buf);
        let out = read_frame(&mut cursor).await.unwrap();
        assert_eq!(out.len(), payload.len());
    }

    #[tokio::test]
    async fn test_binary_payload_preserved() {
        // Ensures no byte stripping occurs on arbitrary payloads including trailing
        // newlines
        let payload = b"{\"jsonrpc\":\"2.0\",\"result\":null,\"id\":1}\n";
        let mut buf = Vec::new();
        write_frame(&mut buf, payload).await.unwrap();
        let mut cursor = Cursor::new(buf);
        let out = read_frame(&mut cursor).await.unwrap();
        assert_eq!(out, payload);
    }
}
