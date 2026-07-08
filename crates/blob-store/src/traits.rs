use async_trait::async_trait;
use zeroize::Zeroizing;

use crate::errors::BlobError;

/// A content-addressed, per-service blob backend. Implementations are
/// responsible for path-traversal guards, quota enforcement, and (when a
/// DEK is supplied) encryption at rest -- see `object_store_impl.rs` for
/// the concrete `object_store`-backed implementation.
///
/// `dek: Option<Zeroizing<[u8; 32]>>` mirrors the rest of M3A: `None` when
/// `storage.encryption = false`, `Some(dek)` otherwise. Callers resolve the
/// DEK once (via `StorageProvider::load_service_dek`) and pass it in --
/// this crate has no dependency on `data-layer`/`key-store`.
#[async_trait]
pub trait BlobProvider: Send + Sync + std::fmt::Debug {
    /// Opens a streaming upload session. The returned hash is only valid
    /// after `UploadSession::finish` succeeds.
    async fn open_upload(
        &self,
        service_id: &str,
        dek: Option<Zeroizing<[u8; 32]>>,
    ) -> Result<Box<dyn UploadSession>, BlobError>;

    /// Opens a streaming download session starting at `offset` bytes into
    /// the plaintext.
    async fn open_download(
        &self,
        service_id: &str,
        hash: &str,
        offset: u64,
        dek: Option<Zeroizing<[u8; 32]>>,
    ) -> Result<Box<dyn DownloadSession>, BlobError>;

    async fn delete_blob(&self, service_id: &str, hash: &str) -> Result<(), BlobError>;

    /// Computes an HMAC-signed URL string for `hash`. Does not itself serve
    /// the blob over HTTP -- no such endpoint exists yet (see status.md).
    async fn signed_url(
        &self,
        service_id: &str,
        hash: &str,
        ttl_secs: u32,
        dek: Option<Zeroizing<[u8; 32]>>,
    ) -> Result<String, BlobError>;

    /// One-shot convenience for small blobs. Default implementation is a
    /// thin wrapper over `open_upload` -- exactly one write, then finish.
    async fn put_blob(
        &self,
        service_id: &str,
        data: Vec<u8>,
        dek: Option<Zeroizing<[u8; 32]>>,
    ) -> Result<String, BlobError> {
        let mut session = self.open_upload(service_id, dek).await?;
        session.write(data).await?;
        session.finish().await
    }

    /// One-shot convenience for small blobs. Default implementation is a
    /// thin wrapper over `open_download` -- reads until EOF.
    async fn get_blob(
        &self,
        service_id: &str,
        hash: &str,
        dek: Option<Zeroizing<[u8; 32]>>,
    ) -> Result<Vec<u8>, BlobError> {
        let mut session = self.open_download(service_id, hash, 0, dek).await?;
        let mut out = Vec::new();
        loop {
            let chunk = session.read(u32::MAX).await?;
            if chunk.is_empty() {
                break;
            }
            out.extend(chunk);
        }
        Ok(out)
    }
}

/// A single in-flight streaming upload. `write` enforces the configured
/// single-blob quota incrementally, so an oversized upload fails fast
/// without buffering the whole thing.
#[async_trait]
pub trait UploadSession: Send {
    async fn write(&mut self, chunk: Vec<u8>) -> Result<(), BlobError>;

    /// Commits the upload and returns the content hash. Consumes the
    /// session -- it cannot be written to afterwards.
    async fn finish(self: Box<Self>) -> Result<String, BlobError>;

    /// Discards a partial upload. Consumes the session; the blob is never
    /// committed. Cheap today (buffer-based, nothing was written to the
    /// backend yet); becomes meaningful backend-side cleanup once true
    /// resumable multipart uploads land.
    async fn abort(self: Box<Self>);
}

/// A single in-flight streaming download.
#[async_trait]
pub trait DownloadSession: Send {
    /// Returns up to `max_bytes`. An empty result signals end of stream.
    async fn read(&mut self, max_bytes: u32) -> Result<Vec<u8>, BlobError>;
}
