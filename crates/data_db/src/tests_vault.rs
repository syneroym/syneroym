//! Vault tests that need only the service store.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use syneroym_data_keystore::KeyStore;
use tempfile::tempdir;

use crate::{SqliteStorageProvider, StorageProvider};

#[tokio::test]
async fn test_vault_delete_secret() {
    let dir = tempdir().unwrap();
    let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([21u8; 32]).unwrap();
    let store = provider.open_service_db("vault-delete", &key_store).await.unwrap();

    store.write_secret("api_key", b"token").await.unwrap();
    store.write_secret("other", b"kept").await.unwrap();
    assert!(store.reveal_secret("api_key").await.unwrap().is_some());

    store.delete_secret("api_key").await.unwrap();
    assert_eq!(store.reveal_secret("api_key").await.unwrap(), None);
    assert_eq!(store.reveal_secret("other").await.unwrap(), Some(b"kept".to_vec()));

    // Deleting a name that is not there is not an error.
    store.delete_secret("api_key").await.unwrap();
    store.delete_secret("never-written").await.unwrap();
}
