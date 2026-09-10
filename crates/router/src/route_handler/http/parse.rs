use super::*;

pub(super) fn blob_hash_from_path(path: &str) -> Option<&str> {
    let hash = path.strip_prefix("/blobs/")?;
    if hash.is_empty() || hash.contains('/') { None } else { Some(hash) }
}

/// **Cache-Control is chosen by content type, not by path.**
/// `text/html`'s name is stable while its content changes every
/// deploy, so caching it immutably would pin a browser to a stale bundle
/// indefinitely. Everything else gets long-lived immutable caching, correct
/// for the bundler-hashed filenames a real asset pipeline produces.
pub(super) fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|part| part.split_once('='))
        .map(|(k, v)| {
            let key = percent_encoding::percent_decode_str(k).decode_utf8_lossy().to_string();
            let val = percent_encoding::percent_decode_str(v).decode_utf8_lossy().to_string();
            (key, val)
        })
        .collect()
}

/// Maps a `GET`-with-query-string request onto `data-layer::query`'s
/// `query-options` (`filter`/`limit`/`cursor`). `limit` and `cursor` are
/// reserved keys mapped directly onto those fields; every other key becomes
/// an equality clause in the MongoDB-style filter document (`?status=open`
/// -> `{"status": "open"}`), matching `compile_filter`'s own `{field:
/// value}` equality shorthand (`crates/data_db/src/filter.rs`) -- string
/// values only, no operators (`$gt`, `$in`, ...) or type coercion. That
/// covers the common case this bridge is for; a route needing richer
/// filtering than plain-equality-AND can still be reached directly via the
/// JSON-RPC bridge, which takes a filter document verbatim. An absent or
/// empty query string maps to an unfiltered query (`filter: null`),
/// unchanged from before this mapping existed. A non-numeric `limit`
/// produces a `400`-worthy error message rather than being silently dropped.
pub(super) fn query_opts_from_query_string(query: &str) -> result::Result<Value, String> {
    let mut params = parse_query(query);
    let limit = match params.remove("limit") {
        Some(raw) => {
            let n = raw
                .parse::<u32>()
                .map_err(|_| format!("invalid `limit` query parameter: {raw:?}"))?;
            Value::Number(n.into())
        }
        None => Value::Null,
    };
    let cursor = params.remove("cursor").map_or(Value::Null, Value::String);
    let filter = if params.is_empty() {
        Value::Null
    } else {
        let filter_doc: serde_json::Map<String, Value> =
            params.into_iter().map(|(k, v)| (k, Value::String(v))).collect();
        Value::String(serde_json::to_string(&filter_doc).map_err(|e| e.to_string())?)
    };
    Ok(serde_json::json!({"filter": filter, "limit": limit, "cursor": cursor}))
}

/// The `svc/<service-id>/` prefix `namespace_topic` adds is a substrate
/// implementation detail. An SSE subscriber names topics the way the route
/// table does, so the wire carries the service-relative name -- a browser
/// cannot subscribe by a name that embeds the deployment's own DID.
/// A topic that is not in this service's namespace (a cross-service
/// subscription) is passed through whole.
pub(super) fn service_relative_topic<'a>(service_id: &str, topic: &'a str) -> &'a str {
    topic
        .strip_prefix("svc/")
        .and_then(|rest| rest.strip_prefix(service_id))
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(topic)
}

/// Formats one broker-delivered `(topic, payload)` message as an SSE frame.
///
/// Payload is treated as UTF-8 text (lossy) -- every fixture in this repo
/// only ever publishes UTF-8 text payloads, and SSE's `data:` framing is
/// line-oriented, so a payload
/// containing newlines emits multiple `data:` lines.
///
/// Topic replaces `\r` and `\n` with spaces: publisher-supplied topics from
/// `MqttBroker` do not validate characters, and unescaped CR/LF in a
/// single-line `event:` field would allow a publisher to inject fabricated
/// `data:` or `event:` lines into another subscriber's stream.
pub(super) fn guest_request_headers(
    headers: &HeaderMap,
) -> result::Result<Vec<(String, String)>, (StatusCode, String)> {
    let mut out = Vec::new();
    for (name, value) in headers {
        let lower = name.as_str().to_ascii_lowercase();
        if HOST_OWNED_HEADERS.contains(&lower.as_str()) {
            continue;
        }
        let Ok(text) = value.to_str() else { continue };
        if out.len() == MAX_GUEST_REQUEST_HEADERS {
            return Err((
                StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                format!("request has more than {MAX_GUEST_REQUEST_HEADERS} headers"),
            ));
        }
        out.push((lower, text.to_string()));
    }
    Ok(out)
}

/// Internal session marker set in `CallerContext.session.claims` when an HTTP
/// request carries a valid session token issued by the local auth service.
pub(super) const SESSION_CALLER_MARKER: &str = "__syneroym_session_caller";

/// The router's view of `CallerContext` as a guest may see it. `Err` on a
/// substrate-injected `AuthLevel`, which cannot legitimately reach an
/// inbound HTTP request -- fail closed rather than report a level that
/// isn't true.
///
/// Takes `preamble` as well as `caller` because the two `auth` halves read
/// different sources: `CallerContext.auth` cannot distinguish a verified
/// certificate from an unchallenged pubkey -- `AuthLevel::Delegated`
/// is assigned to *every* verified preamble, including the client gateway's
/// unchallenged node-DID pubkey -- while the preamble's own `delegation`
/// field can, since a malformed certificate is a hard reject before this
/// point. A session caller carrying `SESSION_CALLER_MARKER` from a verified
/// session token presents `CallerAuth::Delegated`. Conversely
/// `preamble.ucan.is_some()` says only that a token was *attached*, not that it
/// verified (`build_caller` fails open on a bad chain), while
/// `CallerContext.auth == AuthLevel::Ucan` is set only on a verified,
/// unrevoked, capability-bearing chain. The two sources are therefore mixed on
/// purpose, one field from each -- collapsing this to a single source would let
/// a caller self-label the stronger `ucan` value with a junk token.
pub(super) fn guest_caller_identity(
    caller: Option<&CallerContext>,
    preamble: &RoutePreamble,
) -> result::Result<Option<CallerIdentity>, String> {
    let Some(caller) = caller else { return Ok(None) };
    if matches!(
        caller.auth,
        AuthLevel::LocalElevated | AuthLevel::LocalReadOnly | AuthLevel::System
    ) {
        return Err("substrate-injected auth level on an inbound HTTP request".to_string());
    }
    let auth = if caller.session.claims.contains_key(SESSION_CALLER_MARKER) {
        CallerAuth::Delegated
    } else if matches!(caller.auth, AuthLevel::Ucan) {
        CallerAuth::Ucan
    } else if preamble.delegation.is_some() {
        CallerAuth::Delegated
    } else {
        CallerAuth::SelfAsserted
    };
    Ok(Some(CallerIdentity {
        did: caller.caller_did.clone(),
        auth,
        app_instance: caller.app_instance.clone(),
    }))
}

pub(super) fn extract_session_token_from_hyper_headers(
    headers: &hyper::HeaderMap,
) -> Option<String> {
    if let Some(cookie) = headers.get(hyper::header::COOKIE).and_then(|v| v.to_str().ok()) {
        for pair in cookie.split(';') {
            let mut parts = pair.splitn(2, '=');
            if let (Some(k), Some(v)) = (parts.next(), parts.next())
                && k.trim() == syneroym_core::protocol_utils::SESSION_COOKIE_NAME
            {
                return Some(v.trim().to_string());
            }
        }
    }
    if let Some(auth) = headers.get(hyper::header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        let trimmed = auth.trim();
        if let Some(token) = trimmed.strip_prefix("Bearer ") {
            return Some(token.trim().to_string());
        }
        if let Some(token) = trimmed.strip_prefix("bearer ") {
            return Some(token.trim().to_string());
        }
    }
    None
}

pub(super) fn resolve_effective_session_caller(
    route_handler: &RouteHandler,
    preamble: &RoutePreamble,
    caller: Option<&CallerContext>,
    headers: &hyper::HeaderMap,
) -> Option<CallerContext> {
    // Only gateway-origin traffic may use session cookies / bearer tokens (Finding
    // 2 & 10)
    let is_gateway_origin = caller.is_some_and(|c| {
        c.caller_did == route_handler.inner.node_did && preamble.delegation.is_none()
    });

    if !is_gateway_origin {
        return None;
    }

    let token_str = extract_session_token_from_hyper_headers(headers)?;

    // Check revocation (Finding 1)
    let is_revoked =
        route_handler.inner.session_revocation.as_ref().is_some_and(|r| r.is_revoked(&token_str));

    if is_revoked {
        return None;
    }

    // Fail closed: if no auth service is configured on this node, reject all
    // session tokens
    let auth_did = route_handler
        .inner
        .native_http
        .get(syneroym_core::protocol_utils::AUTH_SERVICE_ALIAS)
        .and_then(|svc| svc.service_id().map(ToString::to_string))?;

    let claims = syneroym_ucan::SessionToken::verify(&token_str, &auth_did).ok()?;

    let mut session = caller.map(|c| c.session.clone()).unwrap_or_default();
    session.subject_did = claims.person_did.clone();
    session.claims.insert(SESSION_CALLER_MARKER.to_string(), serde_json::Value::Bool(true));

    Some(CallerContext {
        caller_did: claims.person_did,
        auth: syneroym_rpc::AuthLevel::Delegated,
        app_instance: None,
        session,
        proof: None,
    })
}
