//! Resolves `(directory, dek)` for one service's async database
//! (`async.db`). The three per-service stores in this crate --
//! [`crate::proxy_outbox::ProxyOutbox`], [`crate::call_dedup::CallDedupGuard`],
//! and [`crate::saga::SagaStore`] -- all need exactly this and nothing else;
//! the caching and single-flighting stay with each owner, because what they
//! cache differs.

use std::{path::PathBuf, sync::Arc};

use syneroym_data_db::StorageProvider;
use syneroym_data_keystore::KeyStore;
use zeroize::Zeroizing;

/// `anyhow`, not [`syneroym_rpc::ProxyError`]: each owner maps a failure
/// here into its own refusal (`ProxyOutbox`/`SagaStore` treat an
/// unavailable DEK as `Internal`; `CallDedupGuard` treats it as
/// `PermissionDenied` -- fail-closed, per its own module doc). Unifying the
/// two lookups is the extraction; unifying what a failure *means* to each
/// caller is not.
pub(crate) async fn async_db_location(
    storage_provider: &Arc<dyn StorageProvider>,
    key_store: &Arc<KeyStore>,
    service_id: &str,
) -> anyhow::Result<(PathBuf, Option<Zeroizing<[u8; 32]>>)> {
    let dek = storage_provider.load_service_dek(service_id, key_store).await?;
    let dir = storage_provider.service_db_dir(service_id)?;
    Ok((dir, dek))
}
