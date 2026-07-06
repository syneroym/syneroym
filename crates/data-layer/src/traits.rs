use std::sync::Arc;

use async_trait::async_trait;
use syneroym_key_store::KeyStore;

use crate::wit_store;

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

    /// Returns whether a service's database already exists on disk, without
    /// creating it. Used to decide whether a deploy is a first deploy (no
    /// existing state) or a re-deploy (existing state), which in turn decides
    /// whether the guest's `init()` or `migrate()` lifecycle hook is invoked.
    async fn service_exists(&self, service_id: &str) -> anyhow::Result<bool>;
}

#[async_trait]
pub trait ServiceStore: Send + Sync {
    /// Inserts a secret value into the service's private vault.
    async fn write_secret(&self, key: &str, secret_bytes: &[u8]) -> anyhow::Result<()>;

    /// Retrieves a secret value from the service's private vault.
    async fn reveal_secret(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>>;

    /// Creates a collection (table) plus any requested field indexes.
    async fn create_collection(
        &self,
        schema: &wit_store::CollectionSchema,
    ) -> Result<(), wit_store::DataLayerError>;

    /// Drops a collection (table) if it exists.
    async fn drop_collection(&self, name: &str) -> Result<(), wit_store::DataLayerError>;

    /// Executes raw DDL. Callers must have already verified the invocation is
    /// happening in a lifecycle (`init`/`migrate`) context -- this method
    /// trusts its caller and always attempts execution after a syntax check.
    async fn execute_ddl(&self, sql: &str) -> Result<(), wit_store::DataLayerError>;

    /// Upserts a record. `creator_id` is supplied by the host and is always
    /// authoritative -- the WIT `record-write-value` has no `creator_id`
    /// field, so there is no guest-controlled value to override.
    async fn put(
        &self,
        collection: &str,
        value: &wit_store::RecordWriteValue,
        creator_id: &str,
    ) -> Result<(), wit_store::DataLayerError>;

    /// Applies an RFC 7396 JSON merge-patch to an existing record's payload.
    async fn patch(
        &self,
        collection: &str,
        id: &str,
        patch_json: &[u8],
    ) -> Result<(), wit_store::DataLayerError>;

    /// Fetches a record by id. Returns `Ok(None)` if the record does not
    /// exist -- a missing record is a valid state, not an error.
    async fn get(
        &self,
        collection: &str,
        id: &str,
    ) -> Result<Option<wit_store::RecordReadValue>, wit_store::DataLayerError>;

    /// Queries records matching an optional MongoDB-style JSON filter, with
    /// cursor pagination. Returns an empty list (not an error) when nothing
    /// matches.
    async fn query(
        &self,
        collection: &str,
        opts: &wit_store::QueryOptions,
    ) -> Result<wit_store::QueryResult, wit_store::DataLayerError>;

    /// Deletes a record by id. Idempotent: deleting a non-existent id is not
    /// an error.
    async fn delete(&self, collection: &str, id: &str) -> Result<(), wit_store::DataLayerError>;

    /// Deletes all records matching an optional filter, returning the number
    /// of affected rows.
    async fn delete_many(
        &self,
        collection: &str,
        filter: Option<&str>,
    ) -> Result<u64, wit_store::DataLayerError>;

    /// Applies all mutations in a single transaction, rolling back entirely
    /// on the first failure.
    async fn batch_mutate(
        &self,
        collection: &str,
        mutations: &[wit_store::Mutation],
        creator_id: &str,
    ) -> Result<(), wit_store::DataLayerError>;
}
