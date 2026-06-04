#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! HTTP proxy component for routing local http client requests to the
//! appropriate substrate within ecosystem

mod gateway;

pub use gateway::ClientGateway;
