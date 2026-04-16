use anyhow::Result;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Writes a length-prefixed frame to the writer.
/// The frame is prefixed with its length as a u32 in big-endian format.
/// This function will trim trailing newlines from the frame before calculating length
/// to maintain compatibility with legacy converters.
pub async fn write_frame<W>(writer: &mut W, mut frame: &[u8]) -> Result<()>
where
    W: AsyncWrite + Unpin + Send,
{
    while frame.last() == Some(&b'\n') || frame.last() == Some(&b'\r') {
        frame = &frame[..frame.len() - 1];
    }

    let len = frame.len() as u32;
    writer.write_u32(len).await?;
    writer.write_all(frame).await?;
    Ok(())
}

/// Reads a length-prefixed frame from the reader.
/// The frame is expected to be prefixed with its length as a u32 in big-endian format.
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
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}
