//! Route preamble parsing and types.
//!
//! A preamble is the prefix of a route that specifies transport, application
//! protocol, interface, and service information.
//!
//! The full preamble grammar is:
//! `<scheme>://<interface>|<service_id>[?<params>]`
//!
//! ## Scheme Overloading
//!
//! To keep the URL compact, well-known scheme tokens overload both the wire
//! transport framing and the application-level protocol initiated by the
//! client:
//!
//! | Preamble Scheme | Wire Transport | Application Protocol | Notes |
//! |-----------------|----------------|----------------------|-------|
//! | `json-rpc://`   | Binary frames  | JSON-RPC 2.0         | Most common JSON-RPC client case |
//! | `http://`       | HTTP/1.1       | JSON-RPC 2.0         | Hyper consumes the stream; payload is JSON-RPC |
//! | `wrpc://`       | Binary frames  | wRPC                 | Native WASM wRPC component protocol (NOTE: not yet implemented) |
//! | `raw://`        | Raw bytes      | Raw bytes            | Used for direct passthrough proxying |
//!
//! ## Composable Schemes
//!
//! Composable schemes split the transport and protocol with a `-` character:
//!
//! | Preamble Scheme | Wire Transport | Application Protocol | Notes |
//! |-----------------|----------------|----------------------|-------|
//! | `http-wrpc://`   | HTTP/1.1       | wRPC                 | NOTE: not yet implemented |
//! | `binary-wrpc://` | Binary frames  | wRPC                 | NOTE: not yet implemented |
//!
//! ## Query Parameters
//!
//! All query parameters are orthogonal to the scheme and apply to any
//! transport unless noted otherwise:
//!
//! | Param | Example | Meaning |
//! |-------|---------|---------|
//! | `enc`/`epk` | `?enc=ecdh-p256&epk=<hex>` | E2E ECDH-P256 handshake key. Orthogonal to transport. |
//! | `pubkey` | `?pubkey=<hex>` | Ed25519 identity key of caller. Must accompany `ts` and `sig`. |
//! | `ts` | `?ts=<unix_seconds>` | Unix timestamp of signature generation (ADR-0002 A1). |
//! | `sig` | `?sig=<128_hex_chars>` | Ed25519 signature over `proof_bytes(service_id, interface, ts)`. |
//! | `on_behalf` | `?on_behalf=<hex_json>` | Original caller's proof passed along a call chain (ADR-0016 A2). |
//! | `delegation` | `?delegation=<hex_json>` | Hex-encoded JSON `DelegationCertificate`. Verified against `pubkey` and master anchor. |
//! | `ucan` | `?ucan=<hex_json>` | Optional UCAN capability token chain. |
//! | `dir` | `?dir=upload\|download` | Stream-protocol direction disambiguator (ADR-0014). |
//!
//! ## Interface Names
//!
//! `preamble.interface` (the `<interface>` segment) is looked up in the
//! target service's `EndpointRegistry` to resolve a `SubstrateEndpoint`. Two
//! disjoint sets of names share this same lookup, which is the source of the
//! `http`/`http-native` naming collision:
//!
//! - **App-declared interfaces**: any WIT interface name (or, for TCP/container
//!   services, a bare capability tag like `http`, e.g. `roymctl svc deploy
//!   --interfaces http --tcp <host:port>`) a deployed component/service
//!   registers for itself at deploy time
//!   (`register_wasm_endpoints`/`ControlPlaneService::deploy`). Fully
//!   app-owner-controlled; the substrate does not reserve or validate these
//!   beyond uniqueness per `(service_id, interface)`.
//! - **Reserved native-capability interfaces** (`NATIVE_CAPABILITY_INTERFACES`,
//!   `syneroym_core::local_registry`): `data-layer`, `vault`, `app-config`,
//!   `blob-store`, `messaging`, `http-native`, `conversation`. Every deployed
//!   service (regardless of `service-type`) automatically gets these registered
//!   as `SubstrateEndpoint::NativeHostChannel` entries pointing at
//!   `SynSvcNativeService` instances.
//!
//! **Authentication status:** A preamble is anonymous only when it has none of
//! `pubkey`, `delegation`, `ucan`, `ts`, `sig`, or `on_behalf`. Every other
//! preamble must pass verification (signature, window, certificate, revocation)
//! or is refused before dispatch.

use std::{convert::Infallible, fmt, result, str::FromStr};

use anyhow::{Result, anyhow};
use fmt::{Display, Formatter};
use syneroym_core::streaming::StreamDirection;
use syneroym_identity::DelegationCertificate;
use syneroym_rpc::{CallerProof, OnBehalfProof, PreambleSignature};
use syneroym_ucan::CapabilityToken;

pub mod proof;
pub mod redacted;
#[cfg(test)]
mod tests;

pub(crate) use proof::check_signature;
#[cfg(test)]
pub(crate) use proof::proof_bytes;
pub use proof::{OutboundIdentity, PROOF_WINDOW_SECS};
pub use redacted::{RedactedPreamble, route_text};

/// Header carried by HTTP/1.1 refusal responses.
pub const REFUSAL_HEADER: &str = "x-syneroym-refusal";

/// Separator used in the routing preamble between the interface name and
/// service ID.
pub const PREAMBLE_SEPARATOR: &str = "|";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteTransport {
    /// Binary framing (length-prefixed frames).
    Binary,
    /// HTTP/1.1 framing.
    Http,
    /// Raw bidirectional bytes (no framing).
    Raw,
}

impl FromStr for RouteTransport {
    type Err = anyhow::Error;

    fn from_str(raw: &str) -> result::Result<Self, Self::Err> {
        match raw {
            "binary" => Ok(Self::Binary),
            "http" => Ok(Self::Http),
            "raw" => Ok(Self::Raw),
            _ => Err(anyhow!("Invalid transport: {raw}")),
        }
    }
}

impl Display for RouteTransport {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary => write!(f, "binary"),
            Self::Http => write!(f, "http"),
            Self::Raw => write!(f, "raw"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteProtocol {
    /// JSON-RPC 2.0 protocol.
    JsonRpc,
    /// wRPC protocol (NOTE: not yet implemented).
    Wrpc,
    /// Raw bytes (no application-level protocol).
    Raw,
    /// Custom or extension protocol.
    Other(String),
}

impl FromStr for RouteProtocol {
    type Err = Infallible;

    fn from_str(raw: &str) -> result::Result<Self, Self::Err> {
        Ok(match raw {
            "json-rpc" => Self::JsonRpc,
            "wrpc" => Self::Wrpc,
            "raw" => Self::Raw,
            other => Self::Other(other.to_string()),
        })
    }
}

impl Display for RouteProtocol {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::JsonRpc => write!(f, "json-rpc"),
            Self::Wrpc => write!(f, "wrpc"),
            Self::Raw => write!(f, "raw"),
            Self::Other(value) => write!(f, "{value}"),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RoutePreamble {
    /// The transport mechanism (e.g., binary, http)
    pub transport: RouteTransport,
    /// The application protocol (e.g., json-rpc, wrpc)
    pub protocol: RouteProtocol,
    /// The interface or namespace for the service (optional, can be empty)
    pub interface: String,
    /// The unique service identifier
    pub service_id: String,
    /// Encryption protocol (e.g., ecdh-p256)
    pub enc: Option<String>,
    /// Ed25519 identity key of the caller (hex)
    pub pubkey: Option<String>,
    /// Client delegation certificate (optional, hex encoded JSON in query
    /// param)
    pub delegation: Option<DelegationCertificate>,
    /// Client UCAN capability token (optional, hex-encoded JSON in the
    /// `ucan=` query param). Verified into `SessionContext` capabilities at
    /// ingress. Mirrors `delegation` in transport shape.
    pub ucan: Option<CapabilityToken>,
    /// Stream direction for a `raw://` stream-protocol request
    /// (ADR-0014's `?dir=upload|download`). `None` for every other
    /// preamble shape; strictly validated (not defaulted) by the router
    /// before a stream-protocol route is served.
    pub dir: Option<StreamDirection>,
    /// Unix seconds when the caller signed (ADR-0002 A1).
    pub ts: Option<u64>,
    /// Ed25519 signature by `pubkey` over `proof_bytes(service_id, interface,
    /// ts)`, lowercase hex.
    pub sig: Option<String>,
    /// The caller's P-256 key for the end-to-end handshake, lowercase hex
    /// SEC1. `pubkey` is only ever the Ed25519 identity key.
    pub epk: Option<String>,
    /// The original caller's proof, passed on by a service in a call chain
    /// (ADR-0016 Amendment 2).
    pub on_behalf: Option<OnBehalfProof>,
}

impl RoutePreamble {
    /// Creates a new preamble with the given route fields and all optional
    /// parameters set to `None`.
    pub fn new(
        transport: RouteTransport,
        protocol: RouteProtocol,
        service_id: impl Into<String>,
        interface: impl Into<String>,
    ) -> Self {
        Self {
            transport,
            protocol,
            interface: interface.into(),
            service_id: service_id.into(),
            enc: None,
            pubkey: None,
            delegation: None,
            ucan: None,
            dir: None,
            ts: None,
            sig: None,
            epk: None,
            on_behalf: None,
        }
    }

    /// Constructs a preamble for the canonical binary JSON-RPC case.
    pub fn binary_json_rpc(service_id: impl Into<String>, interface: impl Into<String>) -> Self {
        Self::new(RouteTransport::Binary, RouteProtocol::JsonRpc, service_id, interface)
    }

    /// Parses a preamble from an HTTP request path of the form
    /// `/v1/{service_id}/{interface}`.
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

        Ok(Self::new(RouteTransport::Http, RouteProtocol::JsonRpc, service_id, interface))
    }

    /// Returns the scheme string for this preamble.
    pub(crate) fn scheme_str(&self) -> String {
        match (self.transport, &self.protocol) {
            (RouteTransport::Http, RouteProtocol::JsonRpc) => "http".to_string(),
            (RouteTransport::Http, p) => format!("http-{p}"),
            (RouteTransport::Binary, RouteProtocol::JsonRpc) => "json-rpc".to_string(),
            (RouteTransport::Binary, RouteProtocol::Wrpc) => "wrpc".to_string(),
            (RouteTransport::Binary, p) => p.to_string(),
            (RouteTransport::Raw, RouteProtocol::Raw) => "raw".to_string(),
            (RouteTransport::Raw, p) => format!("raw-{p}"),
        }
    }

    /// Returns the `scheme://interface|service_id` base string without query
    /// parameters.
    pub(crate) fn route_base(&self) -> String {
        let scheme = self.scheme_str();
        if self.interface.is_empty() {
            format!("{}://{}", scheme, self.service_id)
        } else {
            format!("{}://{}{}{}", scheme, self.interface, PREAMBLE_SEPARATOR, self.service_id)
        }
    }

    /// Returns a redacted log view of this preamble.
    #[must_use]
    pub fn redacted(&self) -> RedactedPreamble<'_> {
        RedactedPreamble(self)
    }

    /// True when any identity field is present on this preamble.
    #[must_use]
    pub fn has_identity_fields(&self) -> bool {
        self.pubkey.is_some()
            || self.delegation.is_some()
            || self.ucan.is_some()
            || self.ts.is_some()
            || self.sig.is_some()
            || self.on_behalf.is_some()
    }

    /// Returns the caller proof if all signed proof fields (`pubkey`, `ts`,
    /// `sig`) are present.
    #[must_use]
    pub fn caller_proof(&self) -> Option<CallerProof> {
        let pubkey_hex = self.pubkey.as_ref()?;
        let ts = self.ts?;
        let sig_hex = self.sig.as_ref()?;
        Some(CallerProof {
            pubkey_hex: pubkey_hex.clone(),
            delegation_json: self.delegation.as_ref().and_then(|d| d.to_json().ok()),
            signed: Some(PreambleSignature {
                service_id: self.service_id.clone(),
                interface: self.interface.clone(),
                ts,
                sig_hex: sig_hex.clone(),
            }),
            on_behalf: self.on_behalf.clone(),
        })
    }

    /// Returns the preamble as a newline-terminated string, suitable for
    /// sending over the wire.
    #[must_use]
    pub fn to_preamble_line(&self) -> String {
        format!("{self}\n")
    }

    /// Parses a preamble string into structured route information.
    ///
    /// # Example
    /// ```
    /// use syneroym_router::{RoutePreamble, RouteProtocol, RouteTransport};
    ///
    /// let p = RoutePreamble::parse("json-rpc://default|service-123").unwrap();
    /// assert_eq!(p.transport, RouteTransport::Binary);
    /// assert_eq!(p.protocol, RouteProtocol::JsonRpc);
    /// assert_eq!(p.interface, "default");
    /// assert_eq!(p.service_id, "service-123");
    /// ```
    pub fn parse(raw: &str) -> Result<Self> {
        let (scheme, target) = raw
            .trim()
            .split_once("://")
            .ok_or_else(|| anyhow!("Invalid preamble format: {}", route_text(raw)))?;

        let (transport, protocol) = parse_scheme(scheme)?;
        let (target_clean, query) =
            if let Some((t, q)) = target.split_once('?') { (t, Some(q)) } else { (target, None) };

        let mut parsed_query = ParsedQuery::default();
        if let Some(q) = query {
            for part in q.split('&') {
                if let Some((k, v)) = part.split_once('=') {
                    parsed_query.parse_param(k, v)?;
                }
            }
        }

        let (interface, service_id) =
            target_clean.rsplit_once(PREAMBLE_SEPARATOR).unwrap_or(("", target_clean));

        if service_id.is_empty() {
            return Err(anyhow!("Incomplete preamble (missing service_id): {}", route_text(raw)));
        }

        Ok(Self {
            transport,
            protocol,
            interface: interface.to_string(),
            service_id: service_id.to_string(),
            enc: parsed_query.enc,
            pubkey: parsed_query.pubkey,
            delegation: parsed_query.delegation,
            ucan: parsed_query.ucan,
            dir: parsed_query.dir,
            ts: parsed_query.ts,
            sig: parsed_query.sig,
            epk: parsed_query.epk,
            on_behalf: parsed_query.on_behalf,
        })
    }
}

fn parse_scheme(scheme: &str) -> Result<(RouteTransport, RouteProtocol)> {
    Ok(match scheme {
        "http" => (RouteTransport::Http, RouteProtocol::JsonRpc),
        "json-rpc" => (RouteTransport::Binary, RouteProtocol::JsonRpc),
        "wrpc" => (RouteTransport::Binary, RouteProtocol::Wrpc),
        "raw" => (RouteTransport::Raw, RouteProtocol::Raw),
        s => {
            if let Some((t_str, p_str)) = s.split_once('-') {
                (t_str.parse()?, p_str.parse().unwrap())
            } else {
                (RouteTransport::Binary, RouteProtocol::Other(s.to_string()))
            }
        }
    })
}

#[derive(Default)]
struct ParsedQuery {
    enc: Option<String>,
    pubkey: Option<String>,
    delegation: Option<DelegationCertificate>,
    ucan: Option<CapabilityToken>,
    dir: Option<StreamDirection>,
    ts: Option<u64>,
    sig: Option<String>,
    epk: Option<String>,
    on_behalf: Option<OnBehalfProof>,
}

impl ParsedQuery {
    fn parse_param(&mut self, k: &str, v: &str) -> Result<()> {
        match k {
            "enc" => self.enc = Some(v.to_string()),
            "epk" => self.epk = Some(v.to_string()),
            "pubkey" => self.pubkey = Some(v.to_string()),
            "delegation" => self.delegation = decode_delegation(v),
            "ucan" => self.ucan = decode_ucan(v),
            "ts" => {
                let parsed = v.parse::<u64>().map_err(|_| anyhow!("invalid ts in preamble"))?;
                self.ts = Some(parsed);
            }
            "sig" => {
                if v.len() != 128
                    || !v.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
                {
                    return Err(anyhow!("invalid sig in preamble"));
                }
                self.sig = Some(v.to_string());
            }
            "on_behalf" => self.on_behalf = Some(decode_on_behalf(v)?),
            "dir" => self.dir = v.parse::<StreamDirection>().ok(),
            _ => {}
        }
        Ok(())
    }
}

fn decode_delegation(v: &str) -> Option<DelegationCertificate> {
    let bytes = hex::decode(v).ok()?;
    let json_str = String::from_utf8(bytes).ok()?;
    DelegationCertificate::from_json(&json_str).ok()
}

fn decode_ucan(v: &str) -> Option<CapabilityToken> {
    let bytes = hex::decode(v).ok()?;
    let json_str = String::from_utf8(bytes).ok()?;
    serde_json::from_str::<CapabilityToken>(&json_str).ok()
}

fn decode_on_behalf(v: &str) -> Result<OnBehalfProof> {
    let bytes = hex::decode(v).map_err(|e| anyhow!("invalid on_behalf hex: {e}"))?;
    let json_str = String::from_utf8(bytes).map_err(|e| anyhow!("invalid on_behalf utf-8: {e}"))?;
    serde_json::from_str::<OnBehalfProof>(&json_str)
        .map_err(|e| anyhow!("invalid on_behalf json: {e}"))
}

impl fmt::Debug for RoutePreamble {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.redacted(), f)
    }
}

impl Display for RoutePreamble {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let base = self.route_base();
        let mut params = Vec::new();
        if let Some(enc) = &self.enc {
            params.push(format!("enc={enc}"));
        }
        if let Some(epk) = &self.epk {
            params.push(format!("epk={epk}"));
        }
        if let Some(pubkey) = &self.pubkey {
            params.push(format!("pubkey={pubkey}"));
        }
        if let Some(delegation) = &self.delegation
            && let Ok(json_str) = delegation.to_json()
        {
            params.push(format!("delegation={}", hex::encode(json_str)));
        }
        if let Some(ucan) = &self.ucan
            && let Ok(json) = serde_json::to_string(ucan)
        {
            params.push(format!("ucan={}", hex::encode(json)));
        }
        if let Some(ts) = self.ts {
            params.push(format!("ts={ts}"));
        }
        if let Some(sig) = &self.sig {
            params.push(format!("sig={sig}"));
        }
        if let Some(on_behalf) = &self.on_behalf
            && let Ok(json) = serde_json::to_string(on_behalf)
        {
            params.push(format!("on_behalf={}", hex::encode(json)));
        }
        if let Some(dir) = &self.dir {
            params.push(format!("dir={dir}"));
        }
        if !params.is_empty() {
            write!(f, "{base}?{}", params.join("&"))
        } else {
            write!(f, "{base}")
        }
    }
}
