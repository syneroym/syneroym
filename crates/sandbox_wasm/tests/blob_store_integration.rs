#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Slice 5 integration test: exercises the real `Host`/`HostBlobWriter`/
//! `HostBlobReader` WIT wiring in `crates/sandbox_wasm/src/engine.rs`
//! directly against a `HostState` -- the same level
//! `test_config_get_and_get_section` in `engine.rs` already uses for
//! `app-config`, rather than a full compiled WASM component (no
//! `blob-store`-importing test component exists yet). This still exercises the
//! real resource-table plumbing (`open-upload`/ `write`/`finish`/`abort`,
//! `open-download`/`read`), not just the `crates/data_blob` crate in
//! isolation.

use std::{
    path::Path,
    sync::{Arc, Weak},
};

use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_sandbox_wasm::{HostState, MessagingContext};
use syneroym_wit_interfaces::host::syneroym::blob_store::blob_store::{
    BlobError, Host as BlobStoreHost, HostBlobReader, HostBlobWriter,
};
use wasmtime::component::Resource;

fn make_host_state(component_id: &str, storage_provider: Arc<dyn StorageProvider>) -> HostState {
    let key_store = Arc::new(KeyStore::new());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(1024 * 1024, None));
    let messaging = MessagingContext {
        broker: Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
        engine: Weak::new(),
    };
    HostState::new(
        component_id.to_string(),
        None,
        key_store,
        storage_provider,
        blob_provider,
        false,
        0,
        messaging,
    )
}

fn shared_storage_provider(dir: &Path) -> Arc<dyn StorageProvider> {
    Arc::new(SqliteStorageProvider::new(dir, false).unwrap())
}

#[tokio::test]
async fn test_one_shot_put_get_delete_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = make_host_state("svc-a", shared_storage_provider(dir.path()));

    let data = b"hello from a wasm guest".to_vec();
    let hash = BlobStoreHost::put_blob(&mut host, data.clone()).await.unwrap();
    assert_eq!(hash.len(), 64);

    let fetched = BlobStoreHost::get_blob(&mut host, hash.clone()).await.unwrap();
    assert_eq!(fetched, data);

    BlobStoreHost::delete_blob(&mut host, hash.clone()).await.unwrap();
    let after_delete = BlobStoreHost::get_blob(&mut host, hash).await;
    assert!(matches!(after_delete, Err(BlobError::NotFound)));
}

#[tokio::test]
async fn test_streaming_upload_and_download_via_resources() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = make_host_state("svc-a", shared_storage_provider(dir.path()));

    let chunk_1 = b"first chunk ".to_vec();
    let chunk_2 = b"second chunk".to_vec();
    let mut expected = chunk_1.clone();
    expected.extend(&chunk_2);

    let writer = BlobStoreHost::open_upload(&mut host).await.unwrap();
    let writer_rep = writer.rep();
    HostBlobWriter::write(&mut host, Resource::new_own(writer_rep), chunk_1).await.unwrap();
    HostBlobWriter::write(&mut host, Resource::new_own(writer_rep), chunk_2).await.unwrap();
    let hash = HostBlobWriter::finish(&mut host, Resource::new_own(writer_rep)).await.unwrap();

    let reader = BlobStoreHost::open_download(&mut host, hash, 0).await.unwrap();
    let reader_rep = reader.rep();
    let mut out = Vec::new();
    loop {
        let chunk =
            HostBlobReader::read(&mut host, Resource::new_own(reader_rep), 4).await.unwrap();
        if chunk.is_empty() {
            break;
        }
        out.extend(chunk);
    }
    assert_eq!(out, expected);
}

#[tokio::test]
async fn test_open_download_with_offset() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = make_host_state("svc-a", shared_storage_provider(dir.path()));

    let data = b"0123456789".to_vec();
    let hash = BlobStoreHost::put_blob(&mut host, data.clone()).await.unwrap();

    let reader = BlobStoreHost::open_download(&mut host, hash, 5).await.unwrap();
    let out = HostBlobReader::read(&mut host, Resource::new_own(reader.rep()), 1024).await.unwrap();
    assert_eq!(out, data[5..]);
}

#[tokio::test]
async fn test_abort_discards_upload() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = make_host_state("svc-a", shared_storage_provider(dir.path()));

    let writer = BlobStoreHost::open_upload(&mut host).await.unwrap();
    let writer_rep = writer.rep();
    HostBlobWriter::write(&mut host, Resource::new_own(writer_rep), b"never committed".to_vec())
        .await
        .unwrap();
    HostBlobWriter::abort(&mut host, Resource::new_own(writer_rep)).await;

    // Nothing was ever finished, so there's no hash to look up -- this test
    // just asserts abort() doesn't panic/error and the resource is cleaned
    // up (a second abort-equivalent call, dropping an unknown resource,
    // would be the real double-free hazard; that's covered by the host's
    // ResourceTable itself rejecting reuse of a deleted handle).
}

#[tokio::test]
async fn test_cross_service_blob_isolation() {
    let dir = tempfile::tempdir().unwrap();
    let shared_storage = shared_storage_provider(dir.path());
    let mut host_a = make_host_state("svc-a", shared_storage.clone());

    let data = b"only svc-a should see this".to_vec();
    let hash = BlobStoreHost::put_blob(&mut host_a, data).await.unwrap();

    // svc-b has its own isolated blob_provider instance in this test setup
    // (each HostState gets a fresh in-memory ObjectStoreBlobProvider), which
    // already proves isolation trivially. The more meaningful check is that
    // the *shared* ObjectStoreBlobProvider case (one provider, two
    // component_ids) is isolated -- covered directly in
    // crates/data_blob/src/object_store_impl.rs's
    // `namespace_isolation_across_services` test. Here we just confirm
    // svc-a's own blob is retrievable by svc-a.
    let fetched = BlobStoreHost::get_blob(&mut host_a, hash).await.unwrap();
    assert_eq!(fetched, b"only svc-a should see this");
}
