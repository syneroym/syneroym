//! The operator verbs of a service with passport access (the relay and the
//! mailbox): deny a passport and list the running leases. They run in the
//! process that owns the lease store, because the store loads its index only
//! when it opens.

use serde_json::json;
use syneroym_passport::{LeaseStore, PassportId, wire::LeaseInfo};
use syneroym_rpc::{NativeInvocation, NativeResponse, RpcError, RpcResult};
use tokio::task;
use tracing::info;

#[cfg(test)]
mod tests;

const NO_PASSPORT_RELAY: &str = "this substrate runs no relay that accepts passports";
const NO_PASSPORT_MAILBOX: &str = "this substrate runs no mailbox";

/// The passport service a verb acts on: the text of the "no store" error,
/// the verb name for parse errors, and the name used in logs.
#[derive(Debug, Clone, Copy)]
pub(super) struct Service {
    absent: &'static str,
    deny_method: &'static str,
    name: &'static str,
}

pub(super) const RELAY: Service =
    Service { absent: NO_PASSPORT_RELAY, deny_method: "relay-passport-deny", name: "relay" };
pub(super) const MAILBOX: Service =
    Service { absent: NO_PASSPORT_MAILBOX, deny_method: "mailbox-passport-deny", name: "mailbox" };

/// Runs the `-deny` or `-leases` verb of `invocation` on `store`.
pub(super) async fn dispatch(
    service: Service,
    store: Option<&LeaseStore>,
    invocation: NativeInvocation,
) -> RpcResult<NativeResponse> {
    if invocation.method.ends_with("-deny") {
        deny(service, store, invocation.params).await
    } else {
        leases(service, store)
    }
}

/// Params: `(passport_id,)`. Answers how many running leases ended (a bare
/// number, the WIT `result<u64, string>`).
pub(super) async fn deny(
    service: Service,
    store: Option<&LeaseStore>,
    params: serde_json::Value,
) -> RpcResult<NativeResponse> {
    let store = store.ok_or_else(|| RpcError::MethodNotFound(service.absent.into()))?.clone();
    let (passport_id,): (PassportId,) = serde_json::from_value(params)
        .map_err(|e| RpcError::InvalidParams(format!("{}: {e}", service.deny_method)))?;
    let id = passport_id.clone();
    let ended = task::spawn_blocking(move || store.deny(&id, syneroym_passport::now_secs()))
        .await
        .map_err(|e| RpcError::InternalError(format!("the deny task failed: {e}")))?
        .map_err(|e| RpcError::InternalError(format!("{e:#}")))?;
    info!(%passport_id, ended, service = service.name, "passport denied");
    Ok(NativeResponse { payload: json!(ended) })
}

/// Answers the running leases as a list of `LeaseInfo`.
pub(super) fn leases(service: Service, store: Option<&LeaseStore>) -> RpcResult<NativeResponse> {
    let store = store.ok_or_else(|| RpcError::MethodNotFound(service.absent.into()))?;
    let leases: Vec<LeaseInfo> =
        store.running_leases(syneroym_passport::now_secs()).iter().map(LeaseInfo::from).collect();
    let payload =
        serde_json::to_value(leases).map_err(|e| RpcError::InternalError(e.to_string()))?;
    Ok(NativeResponse { payload })
}
