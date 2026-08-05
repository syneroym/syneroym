#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Syneroym connection routing and translation library
//!
//! Integrates Iroh, WebRTC, HTTP, and raw-byte channels (wRPC — TODO: not yet
//! implemented) into a unified, alias-aware internal peer network.

mod call_dedup;
mod connection_router;
pub mod handshake;
pub mod net_iroh;
pub mod net_webrtc;
mod preamble;
mod proxy;
mod route_handler;
mod routing;
mod stop_signal;

pub use call_dedup::{ASYNC_DB_NAME, CallDedupGuard, DedupClaim, GuardOutcome, replay_as_result};
pub use connection_router::{ConnectionRouter, SYNEROYM_ALPN};
pub use handshake::{HandshakeVerifier, MasterAnchorResolver, VerifiedIdentity};
pub use preamble::{RoutePreamble, RouteProtocol, RouteTransport};
pub use proxy::{IrohHop, ProxyRouter, RemoteHop};
pub use route_handler::{RouteHandler, RouteHandlerDeps};
pub use routing::{AdaptationStage, EncryptionStage, RoutePipeline, ServiceStage, TransportStage};
