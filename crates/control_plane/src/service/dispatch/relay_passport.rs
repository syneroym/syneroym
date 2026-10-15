//! The operator verbs of a relay with passport access: deny a passport and
//! list the running leases. They run in the process that owns the lease
//! store, because the store loads its index only when it opens.

use serde_json::json;
use syneroym_passport::{LeaseStore, PassportId, wire::LeaseInfo};
use syneroym_rpc::{NativeResponse, RpcError, RpcResult};
use tokio::task;
use tracing::info;

#[cfg(test)]
mod tests;

const NO_PASSPORT_RELAY: &str = "this substrate runs no relay that accepts passports";

/// Params: `(passport_id,)`. Answers how many running leases ended (a bare
/// number, the WIT `result<u64, string>`).
pub(super) async fn deny(
    store: Option<&LeaseStore>,
    params: serde_json::Value,
) -> RpcResult<NativeResponse> {
    let store = store.ok_or_else(|| RpcError::MethodNotFound(NO_PASSPORT_RELAY.into()))?.clone();
    let (passport_id,): (PassportId,) = serde_json::from_value(params)
        .map_err(|e| RpcError::InvalidParams(format!("relay-passport-deny: {e}")))?;
    let id = passport_id.clone();
    let ended = task::spawn_blocking(move || store.deny(&id, syneroym_passport::now_secs()))
        .await
        .map_err(|e| RpcError::InternalError(format!("the deny task failed: {e}")))?
        .map_err(|e| RpcError::InternalError(format!("{e:#}")))?;
    info!(%passport_id, ended, "relay passport denied");
    Ok(NativeResponse { payload: json!(ended) })
}

/// Answers the running leases as a list of `LeaseInfo`.
pub(super) fn leases(store: Option<&LeaseStore>) -> RpcResult<NativeResponse> {
    let store = store.ok_or_else(|| RpcError::MethodNotFound(NO_PASSPORT_RELAY.into()))?;
    let leases: Vec<LeaseInfo> =
        store.running_leases(syneroym_passport::now_secs()).iter().map(LeaseInfo::from).collect();
    let payload =
        serde_json::to_value(leases).map_err(|e| RpcError::InternalError(e.to_string()))?;
    Ok(NativeResponse { payload })
}
