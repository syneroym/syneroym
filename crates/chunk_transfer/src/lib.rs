//! Shared "push/pull `Vec<u8>` chunks until EOF" core, used by both
//! `blob-store`'s `UploadSession`/`DownloadSession` (M03-sss Slice 5) and
//! `syneroym:messaging`'s `stream-sink`/`stream-cursor` (M03B Slice 6B, see
//! `docs/decisions/0014-quic-stream-protocol-routing.md`). One chunking loop
//! implementation instead of two parallel ones for the same mechanism.

use anyhow::Result;
use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// A source of `Vec<u8>` chunks, pulled by the caller until exhausted.
#[async_trait]
pub trait ChunkSource: Send {
    /// Returns the next chunk, `Ok(None)` on clean EOF, or `Err` on failure.
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>>;
}

/// A sink `Vec<u8>` chunks are pushed into until the caller signals
/// completion. `finalize`/`abort` consume `self` via `Box<Self>` (not plain
/// `self`) so the trait stays object-safe as `Box<dyn ChunkSink>` -- mirrors
/// `syneroym_data_blob::traits::UploadSession`'s existing receiver shape.
#[async_trait]
pub trait ChunkSink: Send {
    async fn push_chunk(&mut self, data: Vec<u8>) -> Result<()>;

    /// Commits everything pushed so far. Only called after a clean EOF.
    async fn finalize(self: Box<Self>) -> Result<()>;

    /// Discards everything pushed so far. Called instead of `finalize` when
    /// the transfer is aborted (e.g. a read error, or a `push_chunk`
    /// failure) -- never both.
    async fn abort(self: Box<Self>);
}

/// Default chunk size used when reading from an `AsyncRead` source in
/// [`push_until_eof`]. `ChunkSource` implementations that already produce
/// their own chunk boundaries (e.g. a guest's `stream-cursor`) are unaffected
/// -- this constant only governs the byte-stream-to-chunks direction.
const DEFAULT_CHUNK_SIZE: usize = 64 * 1024;

/// Pulls chunks from `source` until `Ok(None)`, writing each to `dest` in
/// order. Propagates the first error from either side and stops; does not
/// attempt partial cleanup of `dest` -- that is the caller's responsibility
/// (mirrors `blob-store`'s existing "abort discards, no backend rollback"
/// posture for the analogous case).
pub async fn pull_until_eof<S, W>(mut source: S, mut dest: W) -> Result<()>
where
    S: ChunkSource,
    W: AsyncWrite + Unpin,
{
    while let Some(chunk) = source.next_chunk().await? {
        dest.write_all(&chunk).await?;
    }
    dest.flush().await?;
    Ok(())
}

/// Reads `src` in fixed-size chunks, pushing each into `sink` via
/// `push_chunk`, until `src` reaches EOF. On clean EOF, calls
/// `sink.finalize()`. On any read error or `push_chunk` failure, calls
/// `sink.abort()` instead and returns the error -- `finalize` and `abort` are
/// mutually exclusive, and exactly one is always called.
pub async fn push_until_eof<R>(mut src: R, sink: Box<dyn ChunkSink>) -> Result<()>
where
    R: AsyncRead + Unpin,
{
    let mut sink = sink;
    let mut buf = vec![0u8; DEFAULT_CHUNK_SIZE];
    loop {
        let n = match src.read(&mut buf).await {
            Ok(n) => n,
            Err(e) => {
                sink.abort().await;
                return Err(e.into());
            }
        };
        if n == 0 {
            break;
        }
        if let Err(e) = sink.push_chunk(buf[..n].to_vec()).await {
            sink.abort().await;
            return Err(e);
        }
    }
    sink.finalize().await
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::io::duplex;

    use super::*;

    struct VecSource {
        chunks: std::collections::VecDeque<Vec<u8>>,
        fail_after: Option<usize>,
    }

    impl VecSource {
        fn new(chunks: Vec<Vec<u8>>) -> Self {
            Self { chunks: chunks.into(), fail_after: None }
        }

        fn failing(chunks: Vec<Vec<u8>>, fail_after: usize) -> Self {
            Self { chunks: chunks.into(), fail_after: Some(fail_after) }
        }
    }

    #[async_trait]
    impl ChunkSource for VecSource {
        async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>> {
            if let Some(0) = self.fail_after {
                return Err(anyhow::anyhow!("source failure"));
            }
            if let Some(n) = self.fail_after.as_mut() {
                *n -= 1;
            }
            Ok(self.chunks.pop_front())
        }
    }

    #[derive(Default, Clone)]
    struct RecordingSink {
        pushed: Arc<Mutex<Vec<Vec<u8>>>>,
        finalized: Arc<Mutex<bool>>,
        aborted: Arc<Mutex<bool>>,
        fail_push: bool,
    }

    #[async_trait]
    impl ChunkSink for RecordingSink {
        async fn push_chunk(&mut self, data: Vec<u8>) -> Result<()> {
            if self.fail_push {
                return Err(anyhow::anyhow!("push failure"));
            }
            self.pushed.lock().unwrap().push(data);
            Ok(())
        }

        async fn finalize(self: Box<Self>) -> Result<()> {
            *self.finalized.lock().unwrap() = true;
            Ok(())
        }

        async fn abort(self: Box<Self>) {
            *self.aborted.lock().unwrap() = true;
        }
    }

    #[tokio::test]
    async fn pull_until_eof_writes_all_chunks_in_order() {
        let source = VecSource::new(vec![vec![1, 2, 3], vec![4, 5], vec![6]]);
        let mut dest = Vec::new();
        pull_until_eof(source, &mut dest).await.unwrap();
        assert_eq!(dest, vec![1, 2, 3, 4, 5, 6]);
    }

    #[tokio::test]
    async fn pull_until_eof_propagates_source_error() {
        let source = VecSource::failing(vec![vec![1]], 0);
        let mut dest = Vec::new();
        let err = pull_until_eof(source, &mut dest).await.unwrap_err();
        assert!(err.to_string().contains("source failure"));
    }

    #[tokio::test]
    async fn push_until_eof_pushes_all_bytes_and_finalizes_once() {
        let sink = RecordingSink::default();
        let finalized = sink.finalized.clone();
        let pushed = sink.pushed.clone();

        let (mut client, server) = duplex(1024);
        client.write_all(b"hello world").await.unwrap();
        drop(client); // signals EOF to the server-side reader

        push_until_eof(server, Box::new(sink)).await.unwrap();

        assert!(*finalized.lock().unwrap());
        let total: Vec<u8> = pushed.lock().unwrap().iter().flatten().copied().collect();
        assert_eq!(total, b"hello world");
    }

    #[tokio::test]
    async fn push_until_eof_aborts_without_finalize_on_push_failure() {
        let sink = RecordingSink { fail_push: true, ..Default::default() };
        let finalized = sink.finalized.clone();
        let aborted = sink.aborted.clone();

        let (mut client, server) = duplex(1024);
        client.write_all(b"data").await.unwrap();
        drop(client);

        let result = push_until_eof(server, Box::new(sink)).await;
        assert!(result.is_err());
        assert!(*aborted.lock().unwrap());
        assert!(!*finalized.lock().unwrap());
    }

    /// Proves the `ChunkSink::finalize`/`abort` object-safety fix (`self:
    /// Box<Self>`, not plain `self`) actually compiles and works through a
    /// `Box<dyn ChunkSink>` call site, not just the concrete-type path the
    /// other tests already exercise.
    #[tokio::test]
    async fn box_dyn_chunk_sink_call_site_compiles_and_works() {
        let sink: Box<dyn ChunkSink> = Box::new(RecordingSink::default());
        let (mut client, server) = duplex(1024);
        client.write_all(b"boxed").await.unwrap();
        drop(client);
        push_until_eof(server, sink).await.unwrap();
    }
}
