use std::sync::Arc;

use async_trait::async_trait;
use syneroym_data_keystore::KeyStore;
use zeroize::Zeroizing;

use crate::host_store;

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

    /// Resolves (generating on first use) the DEK for `service_id`, without
    /// opening its `ServiceStore`. `Ok(None)` means encryption is disabled
    /// -- a deliberate per-deployment mode, not an error. Lets callers that
    /// need a raw DEK for something other than SQL storage (e.g. blob
    /// content encryption) resolve one without depending on `rusqlite`.
    async fn load_service_dek(
        &self,
        service_id: &str,
        key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Option<Zeroizing<[u8; 32]>>>;

    /// Returns whether a service's database already exists on disk, without
    /// creating it. Used to decide whether a deploy is a first deploy (no
    /// existing state) or a re-deploy (existing state), which in turn decides
    /// whether the guest's `init()` or `migrate()` lifecycle hook is invoked.
    async fn service_exists(&self, service_id: &str) -> anyhow::Result<bool>;

    /// Saves a new configuration generation for a service.
    async fn save_config_generation(
        &self,
        service_id: &str,
        config_blob: &str,
    ) -> anyhow::Result<u64>;

    /// Deletes a specific configuration generation for a service.
    async fn delete_config_generation(
        &self,
        service_id: &str,
        generation: u64,
    ) -> anyhow::Result<()>;

    /// Gets a specific configuration generation for a service.
    async fn get_config_generation(
        &self,
        service_id: &str,
        generation: u64,
    ) -> anyhow::Result<Option<String>>;

    /// Gets the latest configuration generation and its blob for a service.
    async fn get_latest_config_generation(
        &self,
        service_id: &str,
    ) -> anyhow::Result<Option<(u64, String)>>;

    /// Records a guest messaging subscription so it survives a substrate
    /// restart (see `messaging_subscriptions`, M3B Slice 6A / ADR-0010
    /// Finding A1). Idempotent: re-subscribing to the same topic is a
    /// no-op, not an error.
    async fn save_messaging_subscription(
        &self,
        service_id: &str,
        topic: &str,
    ) -> anyhow::Result<()>;

    /// Removes one guest messaging subscription. Idempotent: deleting a
    /// non-existent row is not an error.
    async fn delete_messaging_subscription(
        &self,
        service_id: &str,
        topic: &str,
    ) -> anyhow::Result<()>;

    /// Removes every messaging subscription for a service (called from
    /// `ControlPlaneService::undeploy`'s cleanup).
    async fn delete_all_messaging_subscriptions_for_service(
        &self,
        service_id: &str,
    ) -> anyhow::Result<()>;

    /// Lists every persisted `(service_id, topic)` subscription, replayed
    /// into the broker on substrate startup.
    async fn list_all_messaging_subscriptions(&self) -> anyhow::Result<Vec<(String, String)>>;
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
        schema: &host_store::CollectionSchema,
    ) -> Result<(), host_store::DataLayerError>;

    /// Drops a collection (table) if it exists.
    async fn drop_collection(&self, name: &str) -> Result<(), host_store::DataLayerError>;

    /// Executes raw DDL. Callers must have already verified the invocation is
    /// happening in a lifecycle (`init`/`migrate`) context -- this method
    /// trusts its caller and always attempts execution after a syntax check.
    async fn execute_ddl(&self, sql: &str) -> Result<(), host_store::DataLayerError>;

    /// Upserts a record. `creator_id` is supplied by the host and is always
    /// authoritative -- the WIT `record-write-value` has no `creator_id`
    /// field, so there is no guest-controlled value to override.
    async fn put(
        &self,
        collection: &str,
        value: &host_store::RecordWriteValue,
        creator_id: &str,
    ) -> Result<(), host_store::DataLayerError>;

    /// Applies an RFC 7396 JSON merge-patch to an existing record's payload.
    async fn patch(
        &self,
        collection: &str,
        id: &str,
        patch_json: &[u8],
    ) -> Result<(), host_store::DataLayerError>;

    /// Fetches a record by id. Returns `Ok(None)` if the record does not
    /// exist -- a missing record is a valid state, not an error.
    async fn get(
        &self,
        collection: &str,
        id: &str,
    ) -> Result<Option<host_store::RecordReadValue>, host_store::DataLayerError>;

    /// Queries records matching an optional MongoDB-style JSON filter, with
    /// cursor pagination. Returns an empty list (not an error) when nothing
    /// matches.
    async fn query(
        &self,
        collection: &str,
        opts: &host_store::QueryOptions,
    ) -> Result<host_store::QueryResult, host_store::DataLayerError>;

    /// Deletes a record by id. Idempotent: deleting a non-existent id is not
    /// an error.
    async fn delete(&self, collection: &str, id: &str) -> Result<(), host_store::DataLayerError>;

    /// Deletes all records matching an optional filter, returning the number
    /// of affected rows.
    async fn delete_many(
        &self,
        collection: &str,
        filter: Option<&str>,
    ) -> Result<u64, host_store::DataLayerError>;

    /// Applies all mutations in a single transaction, rolling back entirely
    /// on the first failure.
    async fn batch_mutate(
        &self,
        collection: &str,
        mutations: &[host_store::Mutation],
        creator_id: &str,
    ) -> Result<(), host_store::DataLayerError>;

    /// Executes a privileged read-only raw-SQL query (ADR-0011). Callers must
    /// have already verified the `data-layer/admin` capability -- this method
    /// trusts its caller for authorization but enforces two invariants
    /// itself: (1) `params` are bound via `?` placeholders, never
    /// interpolated; (2) the statement is read-only (a mutating statement
    /// returns `permission-denied`).
    async fn query_raw(
        &self,
        sql: &str,
        params: &[host_store::SqlValue],
    ) -> Result<host_store::RawQueryResult, host_store::DataLayerError>;
}
