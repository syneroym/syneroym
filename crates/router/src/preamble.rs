//! Route preamble parsing and types.
//!
//! A preamble is the prefix of a route that specifies protocol, interface, and service information.
//! It follows the format: `protocol://[interface.]service_id`
//!
//! Example: `json-rpc://health.substrate-123`
//! - Protocol: `json-rpc`
//! - Interface: `health`
//! - Service ID: `substrate-123`
//!
//! The preamble is used by the router to determine how to forward messages and which
//! protocol handler should process the incoming request.

use anyhow::{Result, anyhow};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteProtocol {
    /// JSON-RPC protocol
    JsonRpc,
    /// WebRPC (wRPC) protocol
    Wrpc,
    /// Custom or extension protocol
    Other(String),
}

impl FromStr for RouteProtocol {
    type Err = std::convert::Infallible;

    fn from_str(raw: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match raw {
            "json-rpc" => Self::JsonRpc,
            "wrpc" => Self::Wrpc,
            other => Self::Other(other.to_string()),
        })
    }
}

impl fmt::Display for RouteProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::JsonRpc => write!(f, "json-rpc"),
            Self::Wrpc => write!(f, "wrpc"),
            Self::Other(value) => write!(f, "{}", value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePreamble {
    /// The protocol to use for this route (e.g., json-rpc, wrpc)
    pub protocol: RouteProtocol,
    /// The interface or namespace for the service (optional, can be empty)
    pub interface: String,
    /// The unique service identifier
    pub service_id: String,
}

impl RoutePreamble {
    /// Parses a preamble string into structured route information.
    ///
    /// The preamble format is: `protocol://[interface.]service_id`
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let preamble = RoutePreamble::parse("json-rpc://health.service-123")?;
    /// assert_eq!(preamble.protocol, RouteProtocol::JsonRpc);
    /// assert_eq!(preamble.interface, "health");
    /// assert_eq!(preamble.service_id, "service-123");
    /// ```
    pub fn parse(raw: &str) -> Result<Self> {
        let (protocol, target) = raw
            .trim()
            .split_once("://")
            .ok_or_else(|| anyhow!("Invalid preamble format: {raw}"))?;

        let (interface, service_id) = target.rsplit_once('.').unwrap_or(("", target));

        if protocol.is_empty() || service_id.is_empty() {
            return Err(anyhow!("Incomplete preamble: {raw}"));
        }

        Ok(Self {
            protocol: protocol.parse().map_err(|_| anyhow!("Invalid protocol: {}", protocol))?,
            interface: interface.to_string(),
            service_id: service_id.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_route_preamble() {
        let parsed = RoutePreamble::parse("json-rpc://health.substrate-123\n").unwrap();
        assert_eq!(parsed.protocol, RouteProtocol::JsonRpc);
        assert_eq!(parsed.interface, "health");
        assert_eq!(parsed.service_id, "substrate-123");
    }

    #[test]
    fn parses_route_preamble_no_interface() {
        let parsed = RoutePreamble::parse("json-rpc://substrate-123\n").unwrap();
        assert_eq!(parsed.protocol, RouteProtocol::JsonRpc);
        assert_eq!(parsed.interface, "");
        assert_eq!(parsed.service_id, "substrate-123");
    }

    #[test]
    fn parses_route_preamble_multiple_dots() {
        let parsed = RoutePreamble::parse("json-rpc://com.example.health.substrate-123\n").unwrap();
        assert_eq!(parsed.protocol, RouteProtocol::JsonRpc);
        assert_eq!(parsed.interface, "com.example.health");
        assert_eq!(parsed.service_id, "substrate-123");
    }
}
