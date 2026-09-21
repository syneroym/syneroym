use std::{
    collections::HashMap,
    fmt, fs, mem,
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex},
    time,
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
    crypto::{BlobDecryptor, BlobEncryptor, HEADER_LEN, sign_url},
    errors::BlobError,
    traits::{BlobProvider, DownloadSession, UploadSession},
};

mod download_session;
mod provider;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests;
mod upload_session;

// Real service ids are DIDs (e.g. `did:key:...`), which contain colons;
// `:` is not a path separator on any Rust-supported OS. Mirrors the same
// fix already applied to `SqliteStorageProvider::SERVICE_ID_REGEX` in
// `crates/data_db/src/sqlite.rs`. Neither this charset nor the hash charset
// below permits `.` or `/`, so path traversal is structurally impossible
// from validated input --
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

impl fmt::Debug for ObjectStoreBlobProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
            fs::create_dir_all(&local_root)?;
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
    /// memory, bounded by `max_blob_bytes` -- see this crate's module docs
    /// for why this is an accepted trade-off.
    ciphertext_buf: Vec<u8>,
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
