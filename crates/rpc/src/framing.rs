//! Length-prefixed and chunked frame encoders/decoders
//!
//! Implements framing protocol layers over async buffers, supporting
//! reliable message boundaries in network streams.

use std::io::ErrorKind;

use anyhow::Result;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

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
