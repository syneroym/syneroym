//! Vault tests that need only the service store.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use syneroym_data_keystore::KeyStore;
use tempfile::tempdir;

use crate::{SqliteStorageProvider, StorageProvider, vault_names};

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

async fn plain_store(dir: &std::path::Path, id: &str) -> Box<dyn crate::ServiceStore> {
    let provider = SqliteStorageProvider::new(dir, false).unwrap();
    provider.open_service_db(id, &Arc::new(KeyStore::new())).await.unwrap()
}

/// Guest SQL could copy the ciphertext of a reserved entry to a new name.
/// The entry name is bound into the ciphertext, so the copy does not open.
#[tokio::test]
async fn a_vault_ciphertext_copied_to_another_name_does_not_open() {
    let dir = tempdir().unwrap();
    let store = plain_store(dir.path(), "vault-copy").await;
    store.write_secret(vault_names::MAILBOX_SECRET, b"seed").await.unwrap();

    let db = dir.path().join("services").join("vault-copy").join("state.db");
    rusqlite::Connection::open(db)
        .unwrap()
        .execute(
            "INSERT INTO _vault SELECT 'copy', ciphertext, nonce, updated_at FROM _vault WHERE \
             key = ?1",
            [vault_names::MAILBOX_SECRET],
        )
        .unwrap();

    assert!(store.reveal_secret("copy").await.is_err());
    assert_eq!(
        store.reveal_secret(vault_names::MAILBOX_SECRET).await.unwrap(),
        Some(b"seed".to_vec())
    );
}

#[tokio::test]
async fn guest_ddl_cannot_touch_the_vault_table() {
    let dir = tempdir().unwrap();
    let store = plain_store(dir.path(), "vault-ddl").await;
    store.write_secret("api_key", b"token").await.unwrap();

    for sql in [
        "INSERT INTO _vault SELECT 'x', ciphertext, nonce, updated_at FROM _vault",
        "CREATE TABLE stolen AS SELECT * FROM _vault",
        "DELETE FROM _vault",
        "UPDATE _vault SET key = 'x'",
        "DROP TABLE _vault",
        "ALTER TABLE _vault RENAME TO gone",
        "CREATE INDEX vault_idx ON _vault (key)",
        "SELECT * FROM main._vault",
    ] {
        assert!(store.execute_ddl(sql).await.is_err(), "{sql}");
    }

    // Ordinary DDL still works, and the host's own vault writes are not
    // checked by the guest's authorizer.
    store.execute_ddl("CREATE TABLE IF NOT EXISTS notes (id TEXT PRIMARY KEY)").await.unwrap();
    store.write_secret("later", b"ok").await.unwrap();
    assert_eq!(store.reveal_secret("api_key").await.unwrap(), Some(b"token".to_vec()));
}

/// A trigger body is checked only when a later statement fires it, which is
/// the host's own write. So guest DDL may not create a trigger at all.
#[tokio::test]
async fn guest_ddl_cannot_create_a_trigger_that_changes_the_vault() {
    let dir = tempdir().unwrap();
    let store = plain_store(dir.path(), "vault-trigger").await;
    store.write_secret(vault_names::MAILBOX_SECRET, b"seed").await.unwrap();
    store.execute_ddl("CREATE TABLE notes (id TEXT PRIMARY KEY)").await.unwrap();

    for sql in [
        "CREATE TRIGGER wipe AFTER INSERT ON notes BEGIN DELETE FROM _vault; END",
        "CREATE TEMP TRIGGER wipe AFTER INSERT ON notes BEGIN DELETE FROM _vault; END",
    ] {
        assert!(store.execute_ddl(sql).await.is_err(), "{sql}");
    }

    store.execute_ddl("INSERT INTO notes VALUES ('a')").await.unwrap();
    assert_eq!(
        store.reveal_secret(vault_names::MAILBOX_SECRET).await.unwrap(),
        Some(b"seed".to_vec())
    );
}

#[tokio::test]
async fn guest_raw_queries_cannot_read_the_vault_table() {
    let dir = tempdir().unwrap();
    let store = plain_store(dir.path(), "vault-raw").await;
    store.write_secret("api_key", b"token").await.unwrap();

    let err = store.query_raw("SELECT key, ciphertext FROM _vault", &[]).await.unwrap_err();
    assert!(matches!(err, crate::host_store::DataLayerError::PermissionDenied), "{err:?}");
}

#[tokio::test]
async fn a_collection_cannot_be_named_after_the_vault_table() {
    let dir = tempdir().unwrap();
    let store = plain_store(dir.path(), "vault-name").await;
    let schema =
        crate::host_store::CollectionSchema { name: "_vault".to_string(), indexes: vec![] };
    assert!(store.create_collection(&schema).await.is_err());
    assert!(store.drop_collection("_vault").await.is_err());
    store.write_secret("still-here", b"yes").await.unwrap();
}
