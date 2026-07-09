use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex},
};

use async_trait::async_trait;
use bytes::Bytes;
use futures::{StreamExt, stream::BoxStream};
use object_store::{
    ObjectStore, ObjectStoreExt, PutPayload, local::LocalFileSystem, path::Path as ObjectPath,
};
use regex::Regex;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{
    crypto::{BlobDecryptor, BlobEncryptor, HEADER_LEN},
    errors::BlobError,
    traits::{BlobProvider, DownloadSession, UploadSession},
};

// Real service ids are DIDs (e.g. `did:key:...`), which contain colons;
// `:` is not a path separator on any Rust-supported OS. Mirrors the same
// fix already applied to `SqliteStorageProvider::SERVICE_ID_REGEX` in
// `crates/data_db/src/sqlite.rs` (discovered as a latent bug in Slice
// 3A). Neither this charset nor the hash charset below permits `.` or `/`,
// so path traversal is structurally impossible from validated input --
// stronger than a runtime `Path::join` + `starts_with` check (which is
// still applied for the `LocalFileSystem` backend below as defense in
// depth, but is not the primary guard).
#[allow(clippy::unwrap_used)]
static SERVICE_ID_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_:\-]{1,128}$").unwrap());
#[allow(clippy::unwrap_used)]
static HASH_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]{64}$").unwrap());

fn validate_service_id(service_id: &str) -> Result<(), BlobError> {
    if SERVICE_ID_REGEX.is_match(service_id) {
        Ok(())
    } else {
        Err(BlobError::Internal(format!("invalid service id: {service_id}")))
    }
}

fn validate_hash(hash: &str) -> Result<(), BlobError> {
    if HASH_REGEX.is_match(hash) {
        Ok(())
    } else {
        Err(BlobError::Internal(format!("invalid blob hash: {hash}")))
    }
}

/// Builds the two-level-prefixed object key. Only ever called with
/// already-validated `service_id`/`hash`.
fn object_path(service_id: &str, hash: &str) -> ObjectPath {
    ObjectPath::from(format!("{service_id}/{}/{}", &hash[0..2], &hash[2..]))
}

/// `object_store`-backed `BlobProvider`. A single `Arc<dyn ObjectStore>`
/// makes the backend switchable via configuration (`LocalFileSystem` for
/// dev/tests, `AmazonS3` for production) with no code changes elsewhere.
pub struct ObjectStoreBlobProvider {
    store: Arc<dyn ObjectStore>,
    /// Set only for the `LocalFileSystem` backend; used for an extra
    /// `starts_with` guard on top of the regex validation above.
    local_root: Option<PathBuf>,
    max_blob_bytes: u64,
    max_service_total_bytes: Option<u64>,
    /// Lazily populated (via one `list()` per service on first touch) and
    /// then maintained incrementally. Only consulted when
    /// `max_service_total_bytes` is `Some`.
    usage: Arc<Mutex<HashMap<String, u64>>>,
}

impl std::fmt::Debug for ObjectStoreBlobProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectStoreBlobProvider")
            .field("local_root", &self.local_root)
            .field("max_blob_bytes", &self.max_blob_bytes)
            .field("max_service_total_bytes", &self.max_service_total_bytes)
            .finish_non_exhaustive()
    }
}

impl ObjectStoreBlobProvider {
    /// Local-filesystem-backed provider, for dev/tests.
    pub fn new_local(
        local_root: PathBuf,
        max_blob_bytes: u64,
        max_service_total_bytes: Option<u64>,
    ) -> anyhow::Result<Self> {
        if !local_root.exists() {
            std::fs::create_dir_all(&local_root)?;
        }
        let store = LocalFileSystem::new_with_prefix(&local_root)?;
        Ok(Self {
            store: Arc::new(store),
            local_root: Some(local_root),
            max_blob_bytes,
            max_service_total_bytes,
            usage: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// S3-compatible backend (AWS S3, MinIO, Tigris, R2, etc. via
    /// `endpoint`). Gated behind the `aws` cargo feature -- see the
    /// `object_store`/`digest` version-pin comment in the root `Cargo.toml`
    /// for why it isn't enabled by default. Credentials are resolved from
    /// the standard `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY` environment
    /// variables by `object_store`'s `AmazonS3Builder`, per ADR-0009.
    #[cfg(feature = "aws")]
    pub fn new_s3(
        endpoint: &str,
        bucket: &str,
        region: &str,
        max_blob_bytes: u64,
        max_service_total_bytes: Option<u64>,
    ) -> anyhow::Result<Self> {
        let store = object_store::aws::AmazonS3Builder::from_env()
            .with_endpoint(endpoint)
            .with_bucket_name(bucket)
            .with_region(region)
            .build()?;
        Ok(Self::from_object_store(Arc::new(store), max_blob_bytes, max_service_total_bytes))
    }

    /// Convenience constructor for tests/benches elsewhere in the workspace
    /// that just need *a* working `BlobProvider` without pulling in
    /// `object_store` directly as a dependency of their own crate.
    #[must_use]
    pub fn in_memory(max_blob_bytes: u64, max_service_total_bytes: Option<u64>) -> Self {
        Self::from_object_store(
            Arc::new(object_store::memory::InMemory::new()),
            max_blob_bytes,
            max_service_total_bytes,
        )
    }

    /// Wraps an arbitrary pre-built `ObjectStore` (e.g. `InMemory` for
    /// tests, or an `AmazonS3` instance built by a caller behind the `aws`
    /// feature).
    #[must_use]
    pub fn from_object_store(
        store: Arc<dyn ObjectStore>,
        max_blob_bytes: u64,
        max_service_total_bytes: Option<u64>,
    ) -> Self {
        Self {
            store,
            local_root: None,
            max_blob_bytes,
            max_service_total_bytes,
            usage: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Ensures `usage` has an entry for `service_id`, populating it via a
    /// one-time `list()` + sum if this is the first time this service has
    /// been touched since the provider was constructed (i.e. after a
    /// restart). No-op when aggregate quotas are disabled.
    async fn ensure_usage_loaded(&self, service_id: &str) -> Result<(), BlobError> {
        if self.max_service_total_bytes.is_none() {
            return Ok(());
        }
        {
            let usage =
                self.usage.lock().map_err(|_| BlobError::Internal("usage lock poisoned".into()))?;
            if usage.contains_key(service_id) {
                return Ok(());
            }
        }
        let prefix = ObjectPath::from(service_id.to_string());
        let mut total: u64 = 0;
        let mut stream = self.store.list(Some(&prefix));
        while let Some(meta) = stream.next().await {
            let meta = meta.map_err(BlobError::from)?;
            total += meta.size;
        }
        let mut usage =
            self.usage.lock().map_err(|_| BlobError::Internal("usage lock poisoned".into()))?;
        usage.entry(service_id.to_string()).or_insert(total);
        Ok(())
    }

    fn record_usage(&self, service_id: &str, delta: i64) {
        if let Ok(mut usage) = self.usage.lock() {
            let entry = usage.entry(service_id.to_string()).or_insert(0);
            *entry = (i64::try_from(*entry).unwrap_or(i64::MAX) + delta).max(0) as u64;
        }
    }

    /// Defense-in-depth on top of the regex validation in `validate_hash`:
    /// only meaningful for the `LocalFileSystem` backend, where `local_root`
    /// is set.
    fn check_local_path_traversal(&self, service_id: &str, hash: &str) -> Result<(), BlobError> {
        if let Some(local_root) = &self.local_root {
            let resolved = local_root.join(service_id).join(&hash[0..2]).join(&hash[2..]);
            if !resolved.starts_with(local_root) {
                return Err(BlobError::Internal("path traversal rejected".to_string()));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl BlobProvider for ObjectStoreBlobProvider {
    async fn open_upload(
        &self,
        service_id: &str,
        dek: Option<Zeroizing<[u8; 32]>>,
    ) -> Result<Box<dyn UploadSession>, BlobError> {
        validate_service_id(service_id)?;
        self.ensure_usage_loaded(service_id).await?;

        let (encryptor, header) = match dek {
            Some(dek) => {
                let (enc, header) = BlobEncryptor::new(&dek, service_id);
                (Some(enc), header)
            }
            None => (None, Vec::new()),
        };

        Ok(Box::new(ObjectStoreUploadSession {
            store: self.store.clone(),
            service_id: service_id.to_string(),
            max_blob_bytes: self.max_blob_bytes,
            max_service_total_bytes: self.max_service_total_bytes,
            reserved: 0,
            usage: self.usage.clone(),
            plaintext_hasher: Sha256::new(),
            plaintext_len: 0,
            encryptor,
            ciphertext_buf: header,
        }))
    }

    async fn open_download(
        &self,
        service_id: &str,
        hash: &str,
        offset: u64,
        dek: Option<Zeroizing<[u8; 32]>>,
    ) -> Result<Box<dyn DownloadSession>, BlobError> {
        validate_service_id(service_id)?;
        validate_hash(hash)?;
        let path = object_path(service_id, hash);
        self.check_local_path_traversal(service_id, hash)?;

        // Unencrypted blobs have no header/segment framing to walk through,
        // so a nonzero `offset` can be satisfied with a backend-side ranged
        // read instead of transferring (and discarding) everything before
        // it. Trade-off: a ranged read can only be checked against the
        // whole-content hash by hashing from byte 0, which defeats the
        // point, so integrity verification is skipped for this path
        // specifically -- `offset == 0` reads (including every `get_blob`
        // call) and all encrypted reads are unaffected.
        let (stream, verify_full_hash, skip_remaining) = if dek.is_none() && offset > 0 {
            let meta = self.store.head(&path).await.map_err(BlobError::from)?;
            let stream = if offset >= meta.size {
                futures::stream::empty::<object_store::Result<Bytes>>().boxed()
            } else {
                let opts = object_store::GetOptions {
                    range: Some(object_store::GetRange::Offset(offset)),
                    ..Default::default()
                };
                self.store.get_opts(&path, opts).await.map_err(BlobError::from)?.into_stream()
            };
            (stream, false, 0)
        } else {
            let get_result = self.store.get(&path).await.map_err(BlobError::from)?;
            (get_result.into_stream(), true, offset)
        };

        Ok(Box::new(ObjectStoreDownloadSession {
            stream,
            raw_buf: Vec::new(),
            pending_out: Vec::new(),
            decryptor: None,
            header_consumed: dek.is_none(),
            dek,
            service_id: service_id.to_string(),
            expected_hash: hash.to_string(),
            plaintext_hasher: Sha256::new(),
            offset_remaining_to_skip: skip_remaining,
            verify_full_hash,
            eof_reached: false,
            finalized: false,
        }))
    }

    async fn delete_blob(&self, service_id: &str, hash: &str) -> Result<(), BlobError> {
        validate_service_id(service_id)?;
        validate_hash(hash)?;
        let path = object_path(service_id, hash);
        self.check_local_path_traversal(service_id, hash)?;

        let prior_size = self.store.head(&path).await.ok().map(|meta| meta.size);

        match self.store.delete(&path).await {
            Ok(()) => {
                if let Some(size) = prior_size {
                    self.record_usage(service_id, -(size as i64));
                }
            }
            Err(object_store::Error::NotFound { .. }) => {}
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    async fn signed_url(
        &self,
        service_id: &str,
        hash: &str,
        ttl_secs: u32,
        dek: Option<Zeroizing<[u8; 32]>>,
    ) -> Result<String, BlobError> {
        validate_service_id(service_id)?;
        validate_hash(hash)?;
        let path = object_path(service_id, hash);
        self.check_local_path_traversal(service_id, hash)?;
        self.store.head(&path).await.map_err(BlobError::from)?;

        let dek = dek.unwrap_or_default();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| BlobError::Internal(e.to_string()))?
            .as_secs();
        Ok(crate::crypto::sign_url(&dek, service_id, hash, ttl_secs, now))
    }
}

struct ObjectStoreUploadSession {
    store: Arc<dyn ObjectStore>,
    service_id: String,
    max_blob_bytes: u64,
    max_service_total_bytes: Option<u64>,
    /// Bytes this session has speculatively added to the shared `usage` map
    /// so far (via `write`), so `abort`/`finish` can refund them precisely.
    reserved: u64,
    usage: Arc<Mutex<HashMap<String, u64>>>,
    plaintext_hasher: Sha256,
    plaintext_len: u64,
    encryptor: Option<BlobEncryptor>,
    /// Header bytes (if encrypted) followed by ciphertext segments as they
    /// are sealed, or raw plaintext bytes when unencrypted. Buffered in
    /// memory, bounded by `max_blob_bytes` -- see crates/data_blob's
    /// module docs / status.md for why this is an accepted trade-off.
    ciphertext_buf: Vec<u8>,
}

#[async_trait]
impl UploadSession for ObjectStoreUploadSession {
    async fn write(&mut self, chunk: Vec<u8>) -> Result<(), BlobError> {
        let chunk_len = chunk.len() as u64;
        let new_len = self.plaintext_len + chunk_len;
        if new_len > self.max_blob_bytes {
            return Err(BlobError::QuotaExceeded);
        }

        if let Some(max_total) = self.max_service_total_bytes {
            // Check-and-reserve under a single lock acquisition against the
            // live shared counter (not a snapshot taken at `open_upload`),
            // so concurrent uploads for the same service can't each
            // independently max out the budget. Trade-off: since the
            // content hash (and thus whether this upload is really a
            // content-addressed dedup of existing bytes) is only known at
            // `finish`, a duplicate upload started right at the quota
            // boundary can spuriously fail here even though it wouldn't
            // have grown real usage -- `finish` refunds dedup'd bytes after
            // the fact, but can't retroactively un-reject an in-flight
            // write that already hit the boundary.
            let mut usage =
                self.usage.lock().map_err(|_| BlobError::Internal("usage lock poisoned".into()))?;
            let current = *usage.get(&self.service_id).unwrap_or(&0);
            if current + chunk_len > max_total {
                return Err(BlobError::QuotaExceeded);
            }
            *usage.entry(self.service_id.clone()).or_insert(0) += chunk_len;
            self.reserved += chunk_len;
        }

        self.plaintext_hasher.update(&chunk);
        self.plaintext_len = new_len;

        match &mut self.encryptor {
            Some(enc) => self.ciphertext_buf.extend(enc.update(&chunk)?),
            None => self.ciphertext_buf.extend(chunk),
        }
        Ok(())
    }

    async fn finish(mut self: Box<Self>) -> Result<String, BlobError> {
        if let Some(enc) = self.encryptor.take() {
            self.ciphertext_buf.extend(enc.finish()?);
        }
        let hash = hex::encode(self.plaintext_hasher.finalize());
        let path = object_path(&self.service_id, &hash);

        // Content-addressed storage: if a blob with this hash already
        // exists, `put` below silently overwrites identical bytes with no
        // real growth in disk usage, so this session's `write`-time
        // reservation must be refunded rather than double-counted.
        let already_existed = self.store.head(&path).await.is_ok();

        self.store
            .put(&path, PutPayload::from(self.ciphertext_buf))
            .await
            .map_err(BlobError::from)?;

        if already_existed
            && self.reserved > 0
            && let Ok(mut usage) = self.usage.lock()
        {
            let entry = usage.entry(self.service_id.clone()).or_insert(0);
            *entry = entry.saturating_sub(self.reserved);
        }
        Ok(hash)
    }

    async fn abort(mut self: Box<Self>) {
        // Nothing was written to the backend, so refund whatever this
        // session speculatively reserved against the aggregate quota in
        // `write` and drop the buffer.
        if self.reserved > 0
            && let Ok(mut usage) = self.usage.lock()
        {
            let entry = usage.entry(self.service_id.clone()).or_insert(0);
            *entry = entry.saturating_sub(self.reserved);
        }
        self.ciphertext_buf.clear();
    }
}

struct ObjectStoreDownloadSession {
    stream: BoxStream<'static, object_store::Result<Bytes>>,
    /// Bytes pulled from `stream` but not yet consumed (header bytes still
    /// pending, or a partial ciphertext segment).
    raw_buf: Vec<u8>,
    /// Decoded plaintext ready to be returned to the caller, already past
    /// the `offset` skip.
    pending_out: Vec<u8>,
    decryptor: Option<BlobDecryptor>,
    header_consumed: bool,
    dek: Option<Zeroizing<[u8; 32]>>,
    service_id: String,
    expected_hash: String,
    plaintext_hasher: Sha256,
    offset_remaining_to_skip: u64,
    /// `false` for a ranged read starting past byte 0 (see `open_download`):
    /// there's no way to check a suffix against a whole-content hash, so
    /// `verify_hash` is a no-op for that case.
    verify_full_hash: bool,
    eof_reached: bool,
    finalized: bool,
}

impl ObjectStoreDownloadSession {
    /// Hashes and (offset-)filters newly decoded plaintext into
    /// `pending_out`. Hashing always covers the true plaintext from byte 0,
    /// even though bytes before `offset` are never returned to the caller.
    fn feed_plaintext(&mut self, plaintext: Vec<u8>) {
        if plaintext.is_empty() {
            return;
        }
        self.plaintext_hasher.update(&plaintext);
        if self.offset_remaining_to_skip == 0 {
            self.pending_out.extend(plaintext);
            return;
        }
        let skip = self.offset_remaining_to_skip.min(plaintext.len() as u64) as usize;
        self.offset_remaining_to_skip -= skip as u64;
        self.pending_out.extend(&plaintext[skip..]);
    }

    fn verify_hash(&self) -> Result<(), BlobError> {
        if !self.verify_full_hash {
            return Ok(());
        }
        let actual = hex::encode(self.plaintext_hasher.clone().finalize());
        if actual == self.expected_hash {
            Ok(())
        } else {
            Err(BlobError::Internal("integrity check failed".to_string()))
        }
    }
}

#[async_trait]
impl DownloadSession for ObjectStoreDownloadSession {
    async fn read(&mut self, max_bytes: u32) -> Result<Vec<u8>, BlobError> {
        while self.pending_out.len() < max_bytes as usize && !self.eof_reached {
            match self.stream.next().await {
                None => {
                    self.eof_reached = true;
                    if !self.finalized {
                        self.finalized = true;
                        if let Some(dec) = self.decryptor.take() {
                            let tail = dec.finish()?;
                            self.feed_plaintext(tail);
                        } else if !self.header_consumed {
                            // Encrypted blob whose stream ended before a
                            // full header arrived -- corrupt/truncated.
                            return Err(BlobError::Internal("integrity check failed".to_string()));
                        }
                        self.verify_hash()?;
                    }
                }
                Some(Err(e)) => return Err(BlobError::Internal(e.to_string())),
                Some(Ok(bytes)) => {
                    self.raw_buf.extend_from_slice(&bytes);
                    if self.dek.is_some() {
                        if !self.header_consumed {
                            if self.raw_buf.len() < HEADER_LEN {
                                continue;
                            }
                            let header: Vec<u8> = self.raw_buf.drain(..HEADER_LEN).collect();
                            #[allow(clippy::expect_used)]
                            let dek = self.dek.clone().expect("checked is_some above");
                            self.decryptor =
                                Some(BlobDecryptor::new(&dek, &self.service_id, &header)?);
                            self.header_consumed = true;
                        }
                        if !self.raw_buf.is_empty()
                            && let Some(dec) = &mut self.decryptor
                        {
                            let plaintext = dec.update(&self.raw_buf)?;
                            self.raw_buf.clear();
                            self.feed_plaintext(plaintext);
                        }
                    } else {
                        let plaintext: Vec<u8> = self.raw_buf.drain(..).collect();
                        self.feed_plaintext(plaintext);
                    }
                }
            }
        }

        let n = (max_bytes as usize).min(self.pending_out.len());
        Ok(self.pending_out.drain(..n).collect())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use object_store::memory::InMemory;
    use tempfile::tempdir;

    use super::*;

    async fn get_all(
        provider: &ObjectStoreBlobProvider,
        service_id: &str,
        hash: &str,
        dek: Option<Zeroizing<[u8; 32]>>,
    ) -> Result<Vec<u8>, BlobError> {
        provider.get_blob(service_id, hash, dek).await
    }

    fn in_memory_provider(max_blob_bytes: u64, max_total: Option<u64>) -> ObjectStoreBlobProvider {
        ObjectStoreBlobProvider::from_object_store(
            Arc::new(InMemory::new()),
            max_blob_bytes,
            max_total,
        )
    }

    #[tokio::test]
    async fn put_get_round_trip_unencrypted() {
        let provider = in_memory_provider(1024 * 1024, None);
        let data = b"hello blob store".to_vec();
        let hash = provider.put_blob("svc-a", data.clone(), None).await.unwrap();
        assert_eq!(hash.len(), 64);
        let out = get_all(&provider, "svc-a", &hash, None).await.unwrap();
        assert_eq!(out, data);
    }

    #[tokio::test]
    async fn put_get_round_trip_encrypted() {
        let provider = in_memory_provider(1024 * 1024, None);
        let dek = [3u8; 32];
        let data: Vec<u8> = (0..5000).map(|i| (i % 251) as u8).collect();
        let hash =
            provider.put_blob("svc-a", data.clone(), Some(Zeroizing::new(dek))).await.unwrap();
        let out = get_all(&provider, "svc-a", &hash, Some(Zeroizing::new(dek))).await.unwrap();
        assert_eq!(out, data);
    }

    #[tokio::test]
    async fn wrong_dek_fails_integrity_check() {
        let provider = in_memory_provider(1024 * 1024, None);
        let dek = [3u8; 32];
        let data = b"secret content".to_vec();
        let hash = provider.put_blob("svc-a", data, Some(Zeroizing::new(dek))).await.unwrap();
        let wrong_dek = [9u8; 32];
        let result = get_all(&provider, "svc-a", &hash, Some(Zeroizing::new(wrong_dek))).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_missing_blob_returns_not_found() {
        let provider = in_memory_provider(1024 * 1024, None);
        let fake_hash = "0".repeat(64);
        let result = get_all(&provider, "svc-a", &fake_hash, None).await;
        assert_eq!(result.unwrap_err(), BlobError::NotFound);
    }

    #[tokio::test]
    async fn delete_then_get_returns_not_found() {
        let provider = in_memory_provider(1024 * 1024, None);
        let hash = provider.put_blob("svc-a", b"data".to_vec(), None).await.unwrap();
        provider.delete_blob("svc-a", &hash).await.unwrap();
        let result = get_all(&provider, "svc-a", &hash, None).await;
        assert_eq!(result.unwrap_err(), BlobError::NotFound);
    }

    #[tokio::test]
    async fn delete_missing_blob_is_idempotent() {
        let provider = in_memory_provider(1024 * 1024, None);
        let fake_hash = "1".repeat(64);
        assert!(provider.delete_blob("svc-a", &fake_hash).await.is_ok());
    }

    #[tokio::test]
    async fn delete_then_delete_again_does_not_double_decrement_usage() {
        // Usage must only be decremented by the call that actually performed
        // the deletion; a second delete of the same (now-gone) blob hits the
        // `NotFound` branch and must be a no-op against the aggregate quota.
        let provider = in_memory_provider(1024, Some(10));
        let hash = provider.put_blob("svc-a", vec![1u8; 10], None).await.unwrap();
        provider.delete_blob("svc-a", &hash).await.unwrap();
        assert!(provider.delete_blob("svc-a", &hash).await.is_ok());

        // Quota was fully freed by exactly one decrement, not underflowed by
        // two -- a fresh 10-byte upload must fit exactly.
        assert!(provider.put_blob("svc-a", vec![2u8; 10], None).await.is_ok());
    }

    #[tokio::test]
    async fn namespace_isolation_across_services() {
        let provider = in_memory_provider(1024 * 1024, None);
        let hash = provider.put_blob("svc-a", b"only in svc-a".to_vec(), None).await.unwrap();
        let result = get_all(&provider, "svc-b", &hash, None).await;
        assert_eq!(result.unwrap_err(), BlobError::NotFound);
    }

    #[tokio::test]
    async fn service_id_path_traversal_rejected() {
        let provider = in_memory_provider(1024 * 1024, None);
        let result = provider.put_blob("../../etc", b"x".to_vec(), None).await;
        assert!(result.is_err());
        let result = provider.open_download("../x", &"a".repeat(64), 0, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn hash_path_traversal_rejected() {
        let provider = in_memory_provider(1024 * 1024, None);
        let result = provider.get_blob("svc-a", "../../../secret.txt", None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn single_blob_quota_exceeded_fails_fast() {
        let provider = in_memory_provider(10, None);
        let result = provider.put_blob("svc-a", vec![0u8; 11], None).await;
        assert_eq!(result.unwrap_err(), BlobError::QuotaExceeded);
    }

    #[tokio::test]
    async fn single_blob_quota_exceeded_mid_upload() {
        let provider = in_memory_provider(10, None);
        let mut session = provider.open_upload("svc-a", None).await.unwrap();
        session.write(vec![0u8; 5]).await.unwrap();
        let result = session.write(vec![0u8; 10]).await;
        assert_eq!(result.unwrap_err(), BlobError::QuotaExceeded);
    }

    #[tokio::test]
    async fn aggregate_quota_exceeded() {
        let provider = in_memory_provider(1024, Some(15));
        provider.put_blob("svc-a", vec![1u8; 10], None).await.unwrap();
        let result = provider.put_blob("svc-a", vec![2u8; 10], None).await;
        assert_eq!(result.unwrap_err(), BlobError::QuotaExceeded);
    }

    #[tokio::test]
    async fn aggregate_quota_is_per_service() {
        let provider = in_memory_provider(1024, Some(15));
        provider.put_blob("svc-a", vec![1u8; 10], None).await.unwrap();
        // svc-b has its own quota budget, unaffected by svc-a's usage.
        assert!(provider.put_blob("svc-b", vec![2u8; 10], None).await.is_ok());
    }

    #[tokio::test]
    async fn aggregate_quota_rechecks_live_usage_across_concurrent_sessions() {
        // Two sessions opened before either writes: each session's quota
        // check must consult the live shared counter (updated by `write`)
        // rather than a snapshot taken at `open_upload`, so together they
        // cannot exceed the service's aggregate budget.
        let provider = in_memory_provider(1024, Some(15));
        let mut session_a = provider.open_upload("svc-a", None).await.unwrap();
        let mut session_b = provider.open_upload("svc-a", None).await.unwrap();

        session_a.write(vec![1u8; 10]).await.unwrap();
        let result = session_b.write(vec![2u8; 10]).await;
        assert_eq!(result.unwrap_err(), BlobError::QuotaExceeded);
    }

    #[tokio::test]
    async fn abort_refunds_reserved_aggregate_quota() {
        let provider = in_memory_provider(1024, Some(10));
        let mut session = provider.open_upload("svc-a", None).await.unwrap();
        session.write(vec![1u8; 10]).await.unwrap();
        session.abort().await;

        // The aborted session's reservation must be refunded, freeing the
        // full budget back up for a subsequent upload.
        assert!(provider.put_blob("svc-a", vec![2u8; 10], None).await.is_ok());
    }

    #[tokio::test]
    async fn overwriting_identical_content_does_not_double_count_usage() {
        // Budget has enough slack for the duplicate upload's speculative
        // write-time reservation to go through; `finish` must then refund it
        // since `put` only overwrote identical bytes with no real growth.
        let provider = in_memory_provider(1024, Some(25));
        let data = vec![7u8; 10];
        provider.put_blob("svc-a", data.clone(), None).await.unwrap();
        provider.put_blob("svc-a", data, None).await.unwrap();

        // Usage should still read as 10 (one copy), leaving room for 15
        // more; if the duplicate had been left double-counted (20), this
        // would fail with QuotaExceeded.
        assert!(provider.put_blob("svc-a", vec![9u8; 15], None).await.is_ok());
    }

    #[tokio::test]
    async fn abort_discards_partial_upload() {
        let provider = in_memory_provider(1024, None);
        let mut session = provider.open_upload("svc-a", None).await.unwrap();
        session.write(b"partial".to_vec()).await.unwrap();
        session.abort().await;
        // Nothing was ever committed, so listing svc-a's namespace is empty.
        let mut stream = provider.store.list(Some(&ObjectPath::from("svc-a")));
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn open_download_with_offset_returns_suffix_plaintext() {
        let provider = in_memory_provider(1024 * 1024, None);
        let data = b"0123456789abcdefghij".to_vec();
        let hash = provider.put_blob("svc-a", data.clone(), None).await.unwrap();
        let mut session = provider.open_download("svc-a", &hash, 10, None).await.unwrap();
        let out = session.read(1024).await.unwrap();
        assert_eq!(out, data[10..]);
    }

    #[tokio::test]
    async fn open_download_with_offset_at_end_of_plaintext_returns_empty() {
        // A ranged read (`GetRange::Offset`) errors on the backend if the
        // offset is at or past the object's length -- `open_download` must
        // short-circuit to an empty session instead of propagating that.
        let provider = in_memory_provider(1024 * 1024, None);
        let data = b"0123456789".to_vec();
        let hash = provider.put_blob("svc-a", data.clone(), None).await.unwrap();
        let mut session =
            provider.open_download("svc-a", &hash, data.len() as u64, None).await.unwrap();
        let out = session.read(1024).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn open_download_with_offset_past_end_of_plaintext_returns_empty() {
        let provider = in_memory_provider(1024 * 1024, None);
        let data = b"0123456789".to_vec();
        let hash = provider.put_blob("svc-a", data.clone(), None).await.unwrap();
        let mut session =
            provider.open_download("svc-a", &hash, data.len() as u64 + 100, None).await.unwrap();
        let out = session.read(1024).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn open_download_with_offset_returns_suffix_encrypted() {
        let provider = in_memory_provider(1024 * 1024, None);
        let dek = [6u8; 32];
        let data: Vec<u8> =
            (0..(crate::crypto::SEGMENT_SIZE + 500)).map(|i| (i % 256) as u8).collect();
        let hash =
            provider.put_blob("svc-a", data.clone(), Some(Zeroizing::new(dek))).await.unwrap();
        let offset = crate::crypto::SEGMENT_SIZE as u64 + 100;
        let mut session = provider
            .open_download("svc-a", &hash, offset, Some(Zeroizing::new(dek)))
            .await
            .unwrap();
        let mut out = Vec::new();
        loop {
            let chunk = session.read(4096).await.unwrap();
            if chunk.is_empty() {
                break;
            }
            out.extend(chunk);
        }
        assert_eq!(out, data[offset as usize..]);
    }

    #[tokio::test]
    async fn encrypted_bytes_at_rest_do_not_contain_plaintext() {
        let dir = tempdir().unwrap();
        let provider =
            ObjectStoreBlobProvider::new_local(dir.path().to_path_buf(), 1024 * 1024, None)
                .unwrap();
        let dek = [8u8; 32];
        let secret_marker = b"THIS_IS_SECRET_PLAINTEXT_MARKER";
        let hash = provider
            .put_blob("svc-a", secret_marker.to_vec(), Some(Zeroizing::new(dek)))
            .await
            .unwrap();

        // Read the raw file bytes directly from disk, bypassing the
        // provider entirely.
        let path = dir.path().join("svc-a").join(&hash[0..2]).join(&hash[2..]);
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(secret_marker.len()).any(|w| w == secret_marker.as_slice()));

        // But it still round-trips correctly through the provider.
        let out = get_all(&provider, "svc-a", &hash, Some(Zeroizing::new(dek))).await.unwrap();
        assert_eq!(out, secret_marker.to_vec());
    }

    #[tokio::test]
    async fn signed_url_for_missing_blob_is_not_found() {
        let provider = in_memory_provider(1024, None);
        let fake_hash = "2".repeat(64);
        let result = provider.signed_url("svc-a", &fake_hash, 60, None).await;
        assert_eq!(result.unwrap_err(), BlobError::NotFound);
    }

    #[tokio::test]
    async fn signed_url_for_existing_blob_succeeds() {
        let provider = in_memory_provider(1024, None);
        let hash = provider.put_blob("svc-a", b"x".to_vec(), None).await.unwrap();
        let url = provider.signed_url("svc-a", &hash, 60, None).await.unwrap();
        assert!(url.contains(&hash));
    }
}
