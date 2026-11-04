use syneroym_core::protocol_utils::AUTH_SERVICE_ALIAS;
use syneroym_ucan::{SessionToken, SessionTokenClaims};

use super::*;

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
/// certificate from a self-asserted master pubkey -- `AuthLevel::Delegated`
/// is assigned to *every* verified preamble, including a caller's
/// self-signed key without delegation -- while the preamble's own `delegation`
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

/// The values a request offers as a session token: the session cookie first,
/// then `Authorization: Bearer`. An app may send its own `Bearer` token
/// next to the platform cookie, so the first value is not always the session.
fn session_token_candidates(headers: &hyper::HeaderMap) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(cookie) = headers.get(hyper::header::COOKIE).and_then(|v| v.to_str().ok()) {
        for pair in cookie.split(';') {
            let mut parts = pair.splitn(2, '=');
            if let (Some(k), Some(v)) = (parts.next(), parts.next())
                && k.trim() == syneroym_core::protocol_utils::SESSION_COOKIE_NAME
            {
                candidates.push(v.trim().to_string());
            }
        }
    }
    if let Some(auth) = headers.get(hyper::header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        let trimmed = auth.trim();
        if let Some(token) = trimmed.strip_prefix("Bearer ").or(trimmed.strip_prefix("bearer ")) {
            candidates.push(token.trim().to_string());
        }
    }
    candidates
}

/// The first candidate that is not revoked and verifies against the local
/// auth service. A stale cookie falls through to a valid `Bearer`, and an
/// app's own `Bearer` never hides a valid cookie. `None` when no auth
/// service is configured (fail closed).
pub(super) fn select_session_token(
    route_handler: &RouteHandler,
    headers: &hyper::HeaderMap,
) -> Option<(String, SessionTokenClaims)> {
    let auth_did = route_handler
        .inner
        .native_http
        .get(AUTH_SERVICE_ALIAS)
        .and_then(|svc| svc.service_id().map(ToString::to_string))?;

    session_token_candidates(headers).into_iter().find_map(|token| {
        // A revoked token must not authenticate even if it has not expired yet.
        let revoked =
            route_handler.inner.session_revocation.as_ref().is_some_and(|r| r.is_revoked(&token));
        if revoked {
            return None;
        }
        let claims = SessionToken::verify(&token, &auth_did).ok()?;
        Some((token, claims))
    })
}

pub(super) fn resolve_effective_session_caller(
    route_handler: &RouteHandler,
    preamble: &RoutePreamble,
    caller: Option<&CallerContext>,
    headers: &hyper::HeaderMap,
) -> Option<CallerContext> {
    // Only gateway-origin traffic may use session cookies / bearer tokens.
    let is_gateway_origin = caller.is_some_and(|c| {
        c.caller_did == route_handler.inner.node_did && preamble.delegation.is_none()
    });

    if !is_gateway_origin {
        return None;
    }

    let (_, claims) = select_session_token(route_handler, headers)?;

    let mut session = caller.map(|c| c.session.clone()).unwrap_or_default();
    session.subject_did = claims.person_did.clone();
    session.claims.insert(SESSION_CALLER_MARKER.to_string(), serde_json::Value::Bool(true));

    Some(CallerContext {
        caller_did: claims.person_did,
        auth: syneroym_rpc::AuthLevel::Delegated,
        app_instance: None,
        session,
        proof: None,
        on_behalf_did: None,
    })
}
