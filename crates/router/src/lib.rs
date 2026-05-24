//! Syneroym connection routing and translation library
//!
//! Integrates Iroh, WebRTC, HTTP, and wRPC channels into a unified, alias-aware
//! internal peer network.

mod connection_router;
pub mod net_iroh;
pub mod net_webrtc;
mod preamble;
mod route_handler;
mod routing;

pub use connection_router::{ConnectionRouter, SYNEROYM_ALPN};
pub use preamble::{RoutePreamble, RouteProtocol, RouteTransport};
pub use routing::{AdaptationStage, EncryptionStage, RoutePipeline, ServiceStage, TransportStage};
