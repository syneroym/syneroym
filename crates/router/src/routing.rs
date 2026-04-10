use syneroym_core::registry::SubstrateEndpoint;

use crate::preamble::RoutePreamble;

/// A resolved route pairs the incoming request descriptor with the currently
/// registered destination endpoint. This stays intentionally small for now:
/// it reflects what the router can reliably know today without modeling every
/// future transport or protocol detail up front.
#[derive(Debug, Clone)]
pub struct ResolvedRoute {
    pub request: RoutePreamble,
    pub endpoint: SubstrateEndpoint,
}

/// High-level delivery shape for the current request.
///
/// This is not meant to freeze the long-term architecture. It is a vocabulary
/// for the concrete cases the router currently understands, and we should feel
/// free to evolve or replace it after we gain more experience with real traffic
/// patterns, protocol adapters, and endpoint types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryMode {
    PassThrough,
    Broker,
    Adapt,
    Reject,
}

/// Known protocol adaptation steps that sit between the incoming request form
/// and the destination endpoint contract.
///
/// We only enumerate adapters we can name concretely today; future request
/// handling may need a richer representation once framing, transport, and
/// application protocol concerns diverge further.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolAdapter {
    JsonRpcToWrpc,
    JsonRpcToPodman,
}

/// Concrete execution plan for a resolved route.
///
/// This is intentionally pragmatic rather than fully general. It gives the code
/// a clearer shape now, while leaving room for a different decomposition later
/// if experience shows that framing, brokering, and adaptation need cleaner
/// separation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteExecution {
    NativeJsonRpc { channel_id: String },
    WasmWrpcPassthrough { channel_id: String },
    Adapted { adapter: ProtocolAdapter },
    Unsupported,
}

/// Small planning object used by the router before touching the stream body.
///
/// The goal is to make request handling easier to read and discuss, not to lock
/// the project into a final abstraction boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingPlan {
    pub delivery_mode: DeliveryMode,
    pub execution: RouteExecution,
}

impl RoutingPlan {
    pub fn unsupported() -> Self {
        Self { delivery_mode: DeliveryMode::Reject, execution: RouteExecution::Unsupported }
    }
}
