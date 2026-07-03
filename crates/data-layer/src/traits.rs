use std::sync::Arc;

use async_trait::async_trait;
use syneroym_key_store::KeyStore;

#[async_trait]
pub trait StorageProvider: Send + Sync {
    /// Opens (and optionally creates/initializes) the isolated, encrypted
    /// SQLite database for a given service.
    async fn open_service_db(
        &self,
        service_id: &str,
        key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Box<dyn ServiceStore>>;

    /// Rotates the KEK, re-encrypting all DEKs in substrate.db.
    async fn rotate_kek(&self, key_store: &Arc<KeyStore>, new_kek: [u8; 32]) -> anyhow::Result<()>;
}

#[async_trait]
pub trait ServiceStore: Send + Sync {
    /// Inserts a secret value into the service's private vault.
    async fn write_secret(&self, key: &str, secret_bytes: &[u8]) -> anyhow::Result<()>;

    /// Retrieves a secret value from the service's private vault.
    async fn reveal_secret(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>>;

    // Placeholders for Slice 3A data operations
    async fn create_collection(&self, name: &str) -> anyhow::Result<()>;
    async fn drop_collection(&self, name: &str) -> anyhow::Result<()>;
    async fn execute_ddl(&self, sql: &str) -> anyhow::Result<()>;
}
