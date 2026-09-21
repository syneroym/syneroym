#![allow(clippy::too_many_lines, clippy::cognitive_complexity)]

use object_store::memory::InMemory;
use tempfile::tempdir;

use super::*;
use crate::crypto::SEGMENT_SIZE;

async fn get_all(
    provider: &ObjectStoreBlobProvider,
    service_id: &str,
    hash: &str,
    dek: Option<Zeroizing<[u8; 32]>>,
) -> Result<Vec<u8>, BlobError> {
    provider.get_blob(service_id, hash, dek).await
}

fn in_memory_provider(max_blob_bytes: u64, max_total: Option<u64>) -> ObjectStoreBlobProvider {
    ObjectStoreBlobProvider::from_object_store(Arc::new(InMemory::new()), max_blob_bytes, max_total)
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
    let hash = provider.put_blob("svc-a", data.clone(), Some(Zeroizing::new(dek))).await.unwrap();
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
async fn dropping_a_session_without_finish_or_abort_still_refunds_reserved_quota() {
    // Neither host build guarantees a caller calls `abort` on a
    // half-finished upload -- an early return on error, or a native
    // caller with no resource-table destructor to fall back on, just
    // drops the writer. `ObjectStoreUploadSession`'s own `Drop` is what
    // has to catch that, not either caller.
    let provider = in_memory_provider(1024, Some(10));
    let mut session = provider.open_upload("svc-a", None).await.unwrap();
    session.write(vec![1u8; 10]).await.unwrap();
    drop(session);

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
    let data: Vec<u8> = (0..(SEGMENT_SIZE + 500)).map(|i| (i % 256) as u8).collect();
    let hash = provider.put_blob("svc-a", data.clone(), Some(Zeroizing::new(dek))).await.unwrap();
    let offset = SEGMENT_SIZE as u64 + 100;
    let mut session =
        provider.open_download("svc-a", &hash, offset, Some(Zeroizing::new(dek))).await.unwrap();
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
        ObjectStoreBlobProvider::new_local(dir.path().to_path_buf(), 1024 * 1024, None).unwrap();
    let dek = [8u8; 32];
    let secret_marker = b"THIS_IS_SECRET_PLAINTEXT_MARKER";
    let hash = provider
        .put_blob("svc-a", secret_marker.to_vec(), Some(Zeroizing::new(dek)))
        .await
        .unwrap();

    // Read the raw file bytes directly from disk, bypassing the
    // provider entirely.
    let path = dir.path().join("svc-a").join(&hash[0..2]).join(&hash[2..]);
    let raw = fs::read(&path).unwrap();
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
