//! Wire protocol and payload utility functions
//!
//! Provides parsing and extraction functions for low-level TLS
//! (SNI/ClientHello) and HTTP (Host headers) protocols to assist routing and
//! address translation.

use anyhow::{Result, anyhow};
use httparse::{EMPTY_HEADER, Request, Status};
use tls_parser::{TlsClientHelloContents, TlsExtension, TlsMessage, TlsMessageHandshake};

#[must_use]
pub fn is_tls_client_hello(buf: &[u8]) -> bool {
    if buf.len() < 5 {
        return false;
    }
    // TLS record header: content type 22 (handshake), version (3, x), length
    buf[0] == 0x16 && buf[1] == 0x03
}

pub fn extract_sni(buf: &[u8]) -> Result<String> {
    match tls_parser::parse_tls_plaintext(buf) {
        Ok((_, record)) => {
            for msg in record.msg {
                if let TlsMessage::Handshake(TlsMessageHandshake::ClientHello(hello)) = msg {
                    return extract_sni_from_hello(&hello);
                }
            }
            Err(anyhow!("No ClientHello found in TLS record"))
        }
        Err(e) => Err(anyhow!("Failed to parse TLS record: {e}")),
    }
}

fn extract_sni_from_hello(hello: &TlsClientHelloContents) -> Result<String> {
    if let Some(extensions) = hello.ext {
        let (_, extensions_list) = tls_parser::parse_tls_extensions(extensions)
            .map_err(|e| anyhow!("Failed to parse TLS extensions: {e}"))?;
        for ext in extensions_list {
            if let TlsExtension::SNI(sni) = ext
                && let Some((_, name)) = sni.into_iter().next()
            {
                return Ok(String::from_utf8_lossy(name).to_string());
            }
        }
    }
    Err(anyhow!("No SNI extension found in ClientHello"))
}

pub fn extract_host_from_http(buf: &[u8]) -> Result<String> {
    let mut headers = [EMPTY_HEADER; 64];
    let mut req = Request::new(&mut headers);
    match req.parse(buf) {
        Ok(Status::Complete(_) | Status::Partial) => {
            for header in req.headers {
                if header.name.eq_ignore_ascii_case("Host") {
                    return Ok(String::from_utf8_lossy(header.value).to_string());
                }
            }
            Err(anyhow!("Host header not found"))
        }
        Err(e) => Err(anyhow!("Failed to parse HTTP request: {e}")),
    }
}

/// The request header carrying a logical service's routing key (ADR-0022
/// §7). Absent means unkeyed. A header rather than a hostname segment
/// because a routing key is unbounded in cardinality and decides nothing
/// about authority -- a wrong one sends a legitimate request to the wrong
/// member, which is what the topology epoch defends against, not a
/// privilege boundary.
pub const ROUTING_KEY_HEADER: &str = "X-Syneroym-Routing-Key";

/// Request-path prefix the client gateway answers itself instead of
/// proxying. Reserved on every gateway hostname, so a deployed service can
/// never own a path under it -- the gateway must be reachable with no
/// service hostname at all, and a browser page on an app's own origin must
/// be able to reach it without a cross-origin request.
pub const GATEWAY_RESERVED_PATH_PREFIX: &str = "/_syneroym/";

/// Cookie carrying a local gateway session token. Consumed and removed by
/// the gateway; it is never forwarded to the target service.
pub const SESSION_COOKIE_NAME: &str = "syneroym_session";

/// The statement a person signs to open a local gateway session. Domain-
/// separated by `typ` so a signature harvested here can never be replayed
/// as a delegation certificate or a UCAN payload, and bound to `node_did`
/// so it cannot be replayed to a different substrate.
///
/// **The exact object matters.** `roymctl` signs it and the gateway
/// re-derives it; RFC-8785 canonicalization sorts keys, so field order in
/// the literal is free, but every key name and the `typ` string must match
/// byte for byte or verification fails with a bare `BadSignature`. Both
/// sides call THIS function -- neither hand-builds the object.
#[must_use]
pub fn gateway_session_assertion(
    node_did: &str,
    nonce: &str,
    person_did: &str,
) -> serde_json::Value {
    serde_json::json!({
        "typ": "syneroym:gateway-session-assertion:v1",
        "node_did": node_did,
        "nonce": nonce,
        "person_did": person_did,
    })
}

/// The exact character width of a `short_hash` (`z32::encode` of a 5-byte
/// SHA-256 prefix), used to tell an `-a`/`-s` segment apart from a nickname
/// segment that merely starts with the same letter (§0.12).
pub(crate) const SHORT_HASH_LEN: usize = 8;

/// What a gateway host names (ADR-0022 §7). One grammar:
///
/// ```text
/// <nickname>-a<app-did-hash>-s<service-name-hash>[-i<interface-hash>]
/// <nickname>-s<service-did-hash>[-i<interface-hash>]
/// ```
///
/// This is the *first* DNS label only -- everything after it (`.localhost`,
/// `.example.com`, or any number of further labels) is ignored, so the
/// scheme is domain-agnostic and carries no format-version marker of its
/// own.
///
/// `-s` always names the service; `-a`'s presence decides whether that is
/// a logical name inside an app (reversed by the app's own supervisor) or
/// a concrete service DID (reversed by the registry's alias index). `-s`
/// is the only required segment; an omitted `-i` means "the service's one
/// app-declared interface", resolved at the destination (D-S3-15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetHost {
    /// A service that belongs to no app instance.
    Service {
        /// `<nickname>-<hash>`, ready to pass to `RegistryClient::lookup`.
        lookup_alias: String,
        /// The `short_hash` of the interface name, or `""` when the `-i`
        /// segment was absent -- "the service's one app-declared
        /// interface", which only the destination can resolve.
        interface: String,
    },
    /// A logical service of an app instance.
    App {
        /// `<nickname>-<app_did_hash>`: the alias the app's Tier-1 record
        /// was admitted under, reconstructed so no new registry surface is
        /// needed. `community_registry::register_endpoint` derives the same
        /// string via `generate_alias`, and the Tier-1 record's `nickname`
        /// is the app instance id.
        app_lookup_alias: String,
        /// Kept beside the alias so a caller can bind the record it gets
        /// back to the host it was asked for -- `RegistryClient::lookup`
        /// cannot check an alias lookup itself, by construction (§0.4).
        app_did_hash: String,
        service_name_hash: String,
        interface: String,
    },
}

impl TargetHost {
    #[must_use]
    pub fn interface(&self) -> &str {
        match self {
            Self::Service { interface, .. } | Self::App { interface, .. } => interface,
        }
    }
}

/// `<nickname>-<hash>`, or a bare `<hash>` when there is no nickname -- the
/// one place the hostname and the registry alias meet (D-S3-2, D-S3-14).
fn join_alias(nickname: &str, hash: &str) -> String {
    if nickname.is_empty() { hash.to_string() } else { format!("{nickname}-{hash}") }
}

/// If the last element of `parts` starts with `prefix` and (when
/// `require_hash_width` is set) is exactly `1 + SHORT_HASH_LEN` characters
/// long, pops it and returns the remainder after the prefix letter.
/// Otherwise leaves `parts` untouched and returns `None`.
///
/// `-a` and `-s` pass `require_hash_width: true`: both always carry a
/// `short_hash`, so requiring the exact width is what stops an optional,
/// last-popped `-a` from misreading a nickname's own final segment as an
/// app hash (§0.12) -- `data-api-s12345678` must not parse as an app host
/// with `app_did_hash = "pi"`. `-i` passes `false` and stays
/// permissive: `EndpointRegistry` resolves an exact interface *name* before
/// a hash, and `docs/developer-guide.md` documents a working
/// `…-iorchestrator` host. That permissiveness is safe only because `-s` is
/// required and popped after `-i`, so `-i` only ever inspects a real `-i`
/// segment or the required `-s`, never a nickname segment.
fn pop_prefixed(parts: &mut Vec<&str>, prefix: char, require_hash_width: bool) -> Option<String> {
    let last = *parts.last()?;
    if !last.starts_with(prefix) || last.len() <= 1 {
        return None;
    }
    if require_hash_width && last.len() != 1 + SHORT_HASH_LEN {
        return None;
    }
    parts.pop();
    Some(last[1..].to_string())
}

/// Parses the host header or SNI into a [`TargetHost`] (ADR-0022 §7).
///
/// Domain-agnostic: everything past the first dot-separated label is
/// ignored (works unchanged for `.localhost`, `.syneroym.net`, or a bare
/// label), after stripping an optional `:<port>` suffix. The first label
/// is parsed right to left; a host whose first label does not carry at
/// least a well-formed `-s<hash>` segment returns `None`.
#[must_use]
pub fn parse_target_host(host: &str) -> Option<TargetHost> {
    let mut host_str = host;
    if let Some((h, p)) = host_str.rsplit_once(':')
        && !p.is_empty()
        && p.chars().all(|c| c.is_ascii_digit())
    {
        host_str = h;
    }

    let label = host_str.split('.').next().unwrap_or(host_str);
    let mut parts: Vec<&str> = label.split('-').collect();

    let interface = pop_prefixed(&mut parts, 'i', false).unwrap_or_default();
    let s_hash = pop_prefixed(&mut parts, 's', true)?;
    let a_hash = pop_prefixed(&mut parts, 'a', true);

    let nickname = parts.join("-");

    Some(match a_hash {
        Some(a_hash) => TargetHost::App {
            app_lookup_alias: join_alias(&nickname, &a_hash),
            app_did_hash: a_hash,
            service_name_hash: s_hash,
            interface,
        },
        None => TargetHost::Service { lookup_alias: join_alias(&nickname, &s_hash), interface },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::short_hash;

    fn h(s: &str) -> String {
        short_hash(s)
    }

    /// Test 60: the `-s`/`-i` form, with a nickname containing dashes, with
    /// no nickname at all, and with an explicit port.
    #[test]
    fn an_unscoped_host_parses_into_an_alias_and_an_interface() {
        let sh = h("did:key:zSvc");
        let ih = h("default");

        let host = format!("my-svc-s{sh}-i{ih}.localhost");
        assert_eq!(
            parse_target_host(&host),
            Some(TargetHost::Service {
                lookup_alias: format!("my-svc-{sh}"),
                interface: ih.clone(),
            })
        );

        let bare = format!("s{sh}-i{ih}.localhost");
        assert_eq!(
            parse_target_host(&bare),
            Some(TargetHost::Service { lookup_alias: sh.clone(), interface: ih.clone() })
        );

        let with_port = format!("my-svc-s{sh}-i{ih}.localhost:7960");
        assert_eq!(
            parse_target_host(&with_port),
            Some(TargetHost::Service { lookup_alias: format!("my-svc-{sh}"), interface: ih })
        );
    }

    /// Test 61.
    #[test]
    fn an_app_scoped_host_parses_into_an_alias_and_three_hashes() {
        let ah = h("did:key:zApp");
        let sh = h("backend");
        let ih = h("default");

        let host = format!("chat-a{ah}-s{sh}-i{ih}.localhost");
        assert_eq!(
            parse_target_host(&host),
            Some(TargetHost::App {
                app_lookup_alias: format!("chat-{ah}"),
                app_did_hash: ah,
                service_name_hash: sh,
                interface: ih,
            })
        );
    }

    /// Test 62: the property the right-to-left grammar exists for.
    #[test]
    fn an_app_scoped_host_with_a_dashed_nickname_reassembles_the_nickname() {
        let ah = h("did:key:zApp");
        let sh = h("backend");

        let host = format!("my-chat-app-a{ah}-s{sh}.localhost");
        assert_eq!(
            parse_target_host(&host),
            Some(TargetHost::App {
                app_lookup_alias: format!("my-chat-app-{ah}"),
                app_did_hash: ah,
                service_name_hash: sh,
                interface: String::new(),
            })
        );
    }

    /// Test 63: §0.12's defect, the one this plan's own first draft
    /// shipped -- a nickname whose final segment starts with `a` must not
    /// be read as an `-a<hash>` app segment.
    #[test]
    fn a_nickname_ending_in_an_a_segment_is_not_read_as_an_app_host() {
        for nickname in ["data-api", "my-app", "chat-admin"] {
            let sh = h("did:key:zSvc");
            let host = format!("{nickname}-s{sh}.localhost");
            assert_eq!(
                parse_target_host(&host),
                Some(TargetHost::Service {
                    lookup_alias: format!("{nickname}-{sh}"),
                    interface: String::new(),
                }),
                "nickname '{nickname}' must not be misread as carrying an app segment"
            );
        }
    }

    /// Test 64: a host missing a well-formed `-s<hash>` segment is refused
    /// -- this is the whole guard now that there is no format marker: an
    /// unrelated host's first label simply never carries one.
    #[test]
    fn a_host_missing_the_service_segment_is_refused() {
        assert_eq!(parse_target_host("my-svc.localhost"), None, "no -s");
        assert_eq!(parse_target_host("my-svc-sabc.localhost"), None, "-s wrong width");
        assert_eq!(parse_target_host("www.example.com"), None, "not a syneroym host");
        assert_eq!(parse_target_host("localhost"), None, "bare localhost");
        assert_eq!(parse_target_host("127.0.0.1"), None, "bare loopback IP");
    }

    /// Test 65: §0.12's asymmetry -- `-i` stays permissive, `-a`/`-s` do
    /// not.
    #[test]
    fn an_interface_segment_may_carry_a_literal_name() {
        let sh = h("did:key:zSvc");
        let host = format!("my-svc-s{sh}-iorchestrator.localhost");
        assert_eq!(
            parse_target_host(&host),
            Some(TargetHost::Service {
                lookup_alias: format!("my-svc-{sh}"),
                interface: "orchestrator".to_string(),
            })
        );
    }

    /// Test 66: the optional `-i` is not an error at the parse -- the
    /// destination resolves it (D-S3-15).
    #[test]
    fn a_host_with_no_interface_segment_parses_with_an_empty_interface() {
        let sh = h("did:key:zSvc");
        assert_eq!(
            parse_target_host(&format!("my-svc-s{sh}.localhost")),
            Some(TargetHost::Service {
                lookup_alias: format!("my-svc-{sh}"),
                interface: String::new(),
            })
        );

        let ah = h("did:key:zApp");
        assert_eq!(
            parse_target_host(&format!("chat-a{ah}-s{sh}.localhost")),
            Some(TargetHost::App {
                app_lookup_alias: format!("chat-{ah}"),
                app_did_hash: ah,
                service_name_hash: sh,
                interface: String::new(),
            })
        );
    }

    /// Test 67: §0.10's claim that the parser is domain-agnostic.
    #[test]
    fn a_domain_other_than_localhost_parses_identically() {
        let sh = h("did:key:zSvc");
        let localhost = parse_target_host(&format!("my-svc-s{sh}.localhost"));
        let real_domain = parse_target_host(&format!("my-svc-s{sh}.syneroym.net"));
        assert_eq!(localhost, real_domain);
        assert!(localhost.is_some());
    }
}
