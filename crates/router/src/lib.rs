#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Syneroym connection routing and translation library
//!
//! Integrates Iroh, WebRTC, HTTP, and raw-byte channels (wRPC — TODO: not yet
//! implemented) into a unified, alias-aware internal peer network.

mod connection_router;
pub mod handshake;
pub mod net_iroh;
pub mod net_webrtc;
mod preamble;
mod route_handler;
mod routing;

pub use connection_router::{ConnectionRouter, SYNEROYM_ALPN};
pub use handshake::{HandshakeVerifier, MasterAnchorResolver, VerifiedIdentity};
pub use preamble::{RoutePreamble, RouteProtocol, RouteTransport};
pub use route_handler::RouteHandler;
pub use routing::{AdaptationStage, EncryptionStage, RoutePipeline, ServiceStage, TransportStage};
