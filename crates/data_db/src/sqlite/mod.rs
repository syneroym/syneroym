mod mutation;
mod provider;
mod query;
mod query_raw;
mod schema;
mod service_store;
mod sieve;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests;

pub use provider::SqliteStorageProvider;
pub(crate) use schema::validate_identifier;
pub use service_store::SqliteServiceStore;
// Re-exports for sibling modules and tests
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use {
    crate::{
        aggregate,
        auth::{QueryAuth, ReadOutcome},
        filter, host_store,
        traits::{ServiceStore, StorageProvider},
    },
    mutation::*,
    provider::*,
    query::*,
    query_raw::*,
    rusqlite::{Connection, Error as SqliteError, params, types::Value as SqlValue},
    schema::*,
    serde_json::{Map, Value},
    service_store::*,
    sieve::*,
    std::sync::{Arc, Mutex},
    syneroym_data_keystore::{KeyStore, KeyStoreError},
    syneroym_fdae::{CompiledSieve, Mode, compile_read},
    syneroym_ucan::Ability,
};

/// Hard upper bound on records returned per `query` page, enforced
/// regardless of what the guest requests via `query-options.limit`.
pub const MAX_QUERY_PAGE_SIZE: u32 = 1000;

/// Hard upper bound on the number of mutations accepted by a single
/// `batch-mutate` call, to bound how long the single-writer actor is
/// occupied by one request.
pub const MAX_BATCH_SIZE: usize = 200;
