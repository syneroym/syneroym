//! The node owner's hook into the home-relay passport of this substrate.

use std::fmt;

use syneroym_passport::wire::{HomeRelaySetRequest, HomeRelayStatus};
use syneroym_rpc::RpcResult;

/// Implemented by the substrate. Set only on a node whose home relay is
/// fixed by `parent_coordinator.iroh.url`.
#[async_trait::async_trait]
pub trait HomeRelayControl: Send + Sync + fmt::Debug {
    /// Check and store the passport, register once when it is active, and
    /// answer the status after that attempt. `InvalidParams` for a request
    /// that cannot be used; `InternalError` when it cannot be stored.
    async fn set_passport(&self, request: HomeRelaySetRequest) -> RpcResult<HomeRelayStatus>;
    /// `None` when no passport is stored.
    fn status(&self) -> Option<HomeRelayStatus>;
}
