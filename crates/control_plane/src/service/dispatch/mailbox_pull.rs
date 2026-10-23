//! Dispatch handler for `security.mailbox-pull-now`.

use std::sync::{Arc, Weak};

use syneroym_rpc::{NativeResponse, RpcError, RpcResult};

use crate::mailbox_pull::MailboxPullControl;

#[cfg(test)]
mod tests;

const NO_RECEIVER: &str = "this substrate runs no mailbox receiver";

/// Params: none (`[]` or `null`). Runs one pull pass, waits for it and
/// answers a `PullSummary`. There is no time limit: the caller's own request
/// timeout ends its wait, and the pass still finishes.
pub(super) async fn pull_now(
    control: Option<&Weak<dyn MailboxPullControl>>,
    params: serde_json::Value,
) -> RpcResult<NativeResponse> {
    let no_params = params.is_null() || params.as_array().is_some_and(Vec::is_empty);
    if !no_params {
        return Err(RpcError::InvalidParams("mailbox-pull-now takes no parameters".into()));
    }
    let puller: Arc<dyn MailboxPullControl> = control
        .and_then(Weak::upgrade)
        .ok_or_else(|| RpcError::MethodNotFound(NO_RECEIVER.into()))?;
    let summary = puller.pull_now().await;
    let payload =
        serde_json::to_value(summary).map_err(|e| RpcError::InternalError(e.to_string()))?;
    Ok(NativeResponse { payload })
}
