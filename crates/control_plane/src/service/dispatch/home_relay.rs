//! Dispatch handlers for node home-relay passport control.

use std::sync::Arc;

use syneroym_passport::wire::HomeRelaySetRequest;
use syneroym_rpc::{NativeResponse, RpcError, RpcResult};

use crate::home_relay::HomeRelayControl;

#[cfg(test)]
mod tests;

const NO_FIXED_RELAY: &str = "this substrate has no fixed home relay: a passport needs \
                              [parent_coordinator.iroh] url. A node that picks its relay from the \
                              registry, or has no relay, cannot use one";

/// Params: `(HomeRelaySetRequest,)`. Answers `HomeRelayStatus`.
pub(super) async fn set(
    control: Option<&Arc<dyn HomeRelayControl>>,
    params: serde_json::Value,
) -> RpcResult<NativeResponse> {
    let control = control.ok_or_else(|| RpcError::MethodNotFound(NO_FIXED_RELAY.into()))?;
    let (request,): (HomeRelaySetRequest,) = serde_json::from_value(params)
        .map_err(|e| RpcError::InvalidParams(format!("home-relay-set: {e}")))?;
    let status = control.set_passport(request).await?;
    to_response(&status)
}

/// Answers `HomeRelayStatus`, or `null` when no passport is stored.
pub(super) fn status(control: Option<&Arc<dyn HomeRelayControl>>) -> RpcResult<NativeResponse> {
    let control = control.ok_or_else(|| RpcError::MethodNotFound(NO_FIXED_RELAY.into()))?;
    to_response(&control.status())
}

fn to_response<T: serde::Serialize>(value: &T) -> RpcResult<NativeResponse> {
    let payload =
        serde_json::to_value(value).map_err(|e| RpcError::InternalError(e.to_string()))?;
    Ok(NativeResponse { payload })
}
