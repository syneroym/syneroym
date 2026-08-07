//! Resolves `(directory, dek)` for one service's async database
//! (`async.db`). The three per-service stores in this crate --
//! [`crate::proxy_outbox::ProxyOutbox`], [`crate::call_dedup::CallDedupGuard`],
//! and [`crate::saga::SagaStore`] -- all need exactly this and nothing else;
//! the caching and single-flighting stay with each owner, because what they
//! cache differs.

use std::{fmt, path::PathBuf, sync::Arc};

use syneroym_data_db::StorageProvider;
use syneroym_data_keystore::KeyStore;
use zeroize::Zeroizing;

/// Which half of [`async_db_location`] failed. Kept distinct rather than
/// flattened to `anyhow::Error`: `load_service_dek` failing means the DEK
/// is unavailable (typically a locked vault), a condition `CallDedupGuard`
/// deliberately fails closed on; `service_db_dir` failing is a path/IO
/// problem with no security meaning at all and must stay retryable. A
/// single `anyhow::Result` here would erase that distinction for every
/// caller, not just the ones that do not need it.
pub(crate) enum AsyncDbLocationError {
    Dek(anyhow::Error),
    Dir(anyhow::Error),
}

impl fmt::Display for AsyncDbLocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dek(e) => write!(f, "{e}"),
            Self::Dir(e) => write!(f, "{e}"),
        }
    }
}

/// `AsyncDbLocationError`, not [`syneroym_rpc::ProxyError`]: each owner maps
/// a failure here into its own refusal (`ProxyOutbox`/`SagaStore` treat
/// either half as `Internal`; `CallDedupGuard` treats `Dek` as
/// `PermissionDenied` -- fail-closed, per its own module doc -- but `Dir` as
/// `Internal`, same as the other two). Unifying the two lookups is the
/// extraction; unifying what a failure *means* to each caller is not.
pub(crate) async fn async_db_location(
    storage_provider: &Arc<dyn StorageProvider>,
    key_store: &Arc<KeyStore>,
    service_id: &str,
) -> Result<(PathBuf, Option<Zeroizing<[u8; 32]>>), AsyncDbLocationError> {
    let dek = storage_provider
        .load_service_dek(service_id, key_store)
        .await
        .map_err(AsyncDbLocationError::Dek)?;
    let dir = storage_provider.service_db_dir(service_id).map_err(AsyncDbLocationError::Dir)?;
    Ok((dir, dek))
}
