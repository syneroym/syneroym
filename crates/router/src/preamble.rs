use anyhow::{Result, anyhow};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteProtocol {
    JsonRpc,
    Wrpc,
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
    pub protocol: RouteProtocol,
    pub interface: String,
    pub service_id: String,
}

impl RoutePreamble {
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
            protocol: protocol.parse().unwrap(),
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
