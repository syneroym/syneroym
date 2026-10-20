#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Syneroym connection routing and translation library
//!
//! Integrates Iroh, WebRTC, HTTP, and raw-byte channels (wRPC — TODO: not yet
//! implemented) into a unified, alias-aware internal peer network.

mod call_dedup;
mod connection_router;
pub mod handshake;
mod home_relay;
pub mod net_iroh;
pub mod net_webrtc;
pub mod peer_records;
mod preamble;
mod proxy;
mod proxy_outbox;
mod route_handler;
mod routing;
mod saga;
mod service_async_db;
mod stop_signal;
#[cfg(test)]
mod test_support;

pub use call_dedup::{ASYNC_DB_NAME, CallDedupGuard, DedupClaim, GuardOutcome, replay_as_result};
pub use connection_router::{ConnectionRouter, SYNEROYM_ALPN};
pub use handshake::{HandshakeVerifier, MasterAnchorResolver, VerifiedIdentity};
pub use home_relay::{HOME_RELAY_CONFIRM_TIMEOUT, HomeRelay, HomeRelayError};
pub use preamble::{
    OutboundIdentity, PREAMBLE_SEPARATOR, PROOF_WINDOW_SECS, REFUSAL_HEADER, RoutePreamble,
    RouteProtocol, RouteTransport, route_text,
};
pub use proxy::{IrohHop, ProxyRouter, ProxyState, RemoteHop};
pub use proxy_outbox::{Disposition, ProxyOutbox, disposition_of, group_key_of};
pub use route_handler::{RouteHandler, RouteHandlerDeps, io::ucan_chain_not_revoked};
pub use routing::{AdaptationStage, EncryptionStage, RoutePipeline, ServiceStage, TransportStage};
pub use saga::SagaStore;
