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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteTransport {
    /// Binary framing (length-prefixed frames).
    Binary,
    /// HTTP/1.1 framing.
    Http,
}

impl FromStr for RouteTransport {
    type Err = anyhow::Error;

    fn from_str(raw: &str) -> std::result::Result<Self, Self::Err> {
        match raw {
            "binary" => Ok(Self::Binary),
            "http" => Ok(Self::Http),
            _ => Err(anyhow!("Invalid transport: {}", raw)),
        }
    }
}

impl fmt::Display for RouteTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary => write!(f, "binary"),
            Self::Http => write!(f, "http"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteProtocol {
    /// JSON-RPC 2.0 protocol.
    JsonRpc,
    /// WebRPC (wRPC) protocol.
    Wrpc,
    /// Custom or extension protocol.
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
    /// The transport mechanism (e.g., binary, http)
    pub transport: RouteTransport,
    /// The application protocol (e.g., json-rpc, wrpc)
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
        let (scheme, target) = raw
            .trim()
            .split_once("://")
            .ok_or_else(|| anyhow!("Invalid preamble format: {raw}"))?;

        // Scheme mapping: well-known aliases take precedence over the combined-scheme
        // splitter so that `json-rpc` and `wrpc` are never mistakenly split on their dash.
        let (transport, protocol) = match scheme {
            "http" => (RouteTransport::Http, RouteProtocol::JsonRpc),
            "json-rpc" => (RouteTransport::Binary, RouteProtocol::JsonRpc),
            "wrpc" => (RouteTransport::Binary, RouteProtocol::Wrpc),
            // Combined schemes such as `http-wrpc`: split on the first `-` that separates
            // a valid transport from a protocol identifier.
            s => {
                if let Some((t_str, p_str)) = s.split_once('-') {
                    (t_str.parse()?, p_str.parse().unwrap())
                } else {
                    (RouteTransport::Binary, RouteProtocol::Other(s.to_string()))
                }
            }
        };

        let (interface, service_id) = target
            .rsplit_once(syneroym_core::constants::PREAMBLE_SEPARATOR)
            .unwrap_or(("", target));

        if service_id.is_empty() {
            return Err(anyhow!("Incomplete preamble (missing service_id): {raw}"));
        }

        Ok(Self {
            transport,
            protocol,
            interface: interface.to_string(),
            service_id: service_id.to_string(),
        })
    }

    /// Constructs a preamble for the canonical binary JSON-RPC case.
    ///
    /// This is the default transport used by `SyneroymClient::request_raw`. Using this
    /// constructor ensures that the framing choice is defined in one place and callers
    /// don't need to know the internal defaults.
    pub fn binary_json_rpc(service_id: impl Into<String>, interface: impl Into<String>) -> Self {
        Self {
            transport: RouteTransport::Binary,
            protocol: RouteProtocol::JsonRpc,
            service_id: service_id.into(),
            interface: interface.into(),
        }
    }

    /// Parses a preamble from an HTTP request path of the form `/v1/{service_id}/{interface}`.
    ///
    /// HTTP is treated purely as a framing concern; the resulting `RouteProtocol` is still
    /// `JsonRpc` so the same routing logic applies regardless of how the bytes arrived.
    pub fn from_http_path(path: &str) -> Result<Self> {
        let rest = path
            .strip_prefix("/v1/")
            .ok_or_else(|| anyhow!("HTTP path must start with /v1/: {path}"))?;

        let (service_id, interface) = rest
            .split_once('/')
            .ok_or_else(|| anyhow!("HTTP path missing interface segment: {path}"))?;

        if service_id.is_empty() || interface.is_empty() {
            return Err(anyhow!("Empty service_id or interface in HTTP path: {path}"));
        }

        Ok(Self {
            transport: RouteTransport::Http,
            protocol: RouteProtocol::JsonRpc,
            interface: interface.to_string(),
            service_id: service_id.to_string(),
        })
    }

    /// Returns the preamble as a newline-terminated string, suitable for sending over the wire.
    ///
    /// This is used by clients to prefix their streams so the router knows how to handle them.
    pub fn to_preamble_line(&self) -> String {
        format!("{}\n", self)
    }
}

impl fmt::Display for RoutePreamble {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scheme = match (self.transport, &self.protocol) {
            (RouteTransport::Http, RouteProtocol::JsonRpc) => "http".to_string(),
            (RouteTransport::Http, p) => format!("http-{}", p),
            (RouteTransport::Binary, RouteProtocol::JsonRpc) => "json-rpc".to_string(),
            (RouteTransport::Binary, RouteProtocol::Wrpc) => "wrpc".to_string(),
            (RouteTransport::Binary, p) => p.to_string(),
        };

        if self.interface.is_empty() {
            write!(f, "{}://{}", scheme, self.service_id)
        } else {
            write!(
                f,
                "{}://{}{}{}",
                scheme,
                self.interface,
                syneroym_core::constants::PREAMBLE_SEPARATOR,
                self.service_id
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_route_preamble() {
        let parsed = RoutePreamble::parse("json-rpc://health|substrate-123\n").unwrap();
        assert_eq!(parsed.transport, RouteTransport::Binary);
        assert_eq!(parsed.protocol, RouteProtocol::JsonRpc);
        assert_eq!(parsed.interface, "health");
        assert_eq!(parsed.service_id, "substrate-123");
    }

    #[test]
    fn parses_http_scheme() {
        let parsed = RoutePreamble::parse("http://health|substrate-123\n").unwrap();
        assert_eq!(parsed.transport, RouteTransport::Http);
        assert_eq!(parsed.protocol, RouteProtocol::JsonRpc);
    }

    #[test]
    fn parses_combined_scheme() {
        let parsed = RoutePreamble::parse("http-wrpc://health|substrate-123\n").unwrap();
        assert_eq!(parsed.transport, RouteTransport::Http);
        assert_eq!(parsed.protocol, RouteProtocol::Wrpc);
    }

    #[test]
    fn parses_route_preamble_no_interface() {
        let parsed = RoutePreamble::parse("json-rpc://substrate-123\n").unwrap();
        assert_eq!(parsed.transport, RouteTransport::Binary);
        assert_eq!(parsed.protocol, RouteProtocol::JsonRpc);
        assert_eq!(parsed.interface, "");
        assert_eq!(parsed.service_id, "substrate-123");
    }

    #[test]
    fn parses_http_path() {
        let parsed = RoutePreamble::from_http_path("/v1/my-service/health").unwrap();
        assert_eq!(parsed.transport, RouteTransport::Http);
        assert_eq!(parsed.protocol, RouteProtocol::JsonRpc);
        assert_eq!(parsed.service_id, "my-service");
        assert_eq!(parsed.interface, "health");
    }

    #[test]
    fn parses_http_path_rejects_bad_prefix() {
        assert!(RoutePreamble::from_http_path("/api/my-service/health").is_err());
    }

    #[test]
    fn parses_http_path_rejects_missing_interface() {
        assert!(RoutePreamble::from_http_path("/v1/my-service").is_err());
    }

    #[test]
    fn parses_http_path_rejects_empty_segments() {
        assert!(RoutePreamble::from_http_path("/v1//health").is_err());
        assert!(RoutePreamble::from_http_path("/v1/my-service/").is_err());
    }

    #[test]
    fn test_display() {
        let p1 = RoutePreamble::parse("http://health|substrate-123").unwrap();
        assert_eq!(p1.to_string(), "http://health|substrate-123");

        let p2 = RoutePreamble::parse("json-rpc://substrate-123").unwrap();
        assert_eq!(p2.to_string(), "json-rpc://substrate-123");

        let p3 = RoutePreamble::parse("http-wrpc://my-service").unwrap();
        assert_eq!(p3.to_string(), "http-wrpc://my-service");

        let p4 = RoutePreamble::parse("wrpc://my-interface|my-service").unwrap();
        assert_eq!(p4.to_string(), "wrpc://my-interface|my-service");
    }
}
