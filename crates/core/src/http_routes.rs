//! The shared per-service HTTP route vocabulary (M3B Slice 7): `HttpRoute`
//! and the `HttpRouteRegistry` table it's kept in.
//!
//! Lives in `core` rather than in `router` or `control_plane` because both
//! sides need it without depending on each other: `syneroym-control-plane`
//! parses these out of a deployed service's `custom_config` on deploy/
//! undeploy (`ControlPlaneService`, `crates/control_plane/src/http_routes.rs`),
//! and `syneroym-router` reads the registry per HTTP request
//! (`crates/router/src/route_handler/http.rs`) to decide how a given verb+
//! path bridges onto `data-layer`/`messaging`/a registered stream protocol/
//! guest code (M06A A2).

use std::sync::Arc;

use dashmap::DashMap;
use serde::Deserialize;

/// One `http_routes` entry. `target` selects which native capability the
/// route bridges onto; the optional fields are only meaningful for the
/// matching target (`collection` for `data-layer`, `topic` for `messaging`,
/// `protocol` for `stream`, `public` for `guest`) and are ignored otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HttpRoute {
    pub method: String,
    pub path: String,
    pub target: String,
    pub operation: String,
    #[serde(default)]
    pub collection: Option<String>,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
    /// Whether a caller with no verified identity may reach this route
    /// (M06A D-A2-7). Only meaningful for `target = "guest"`, where `false`
    /// -- the default -- answers an anonymous request with 401 before the
    /// component is instantiated. Refused at deploy on any other target,
    /// where it would do nothing: `data-layer`/`messaging` already reject
    /// an anonymous caller inside `dispatch_native`, and `stream` predates
    /// this field (M06A §9.5).
    ///
    /// A bool, not ADR-0018's `visibility` enum: this is neither
    /// endpoint-record publication nor byte readability, `syneroym-core`
    /// cannot see `syneroym_app_orchestration::Visibility`, and there is no
    /// middle tier to express.
    #[serde(default)]
    pub public: bool,
}

/// Shared, keyed-by-`service_id` HTTP route table. `ControlPlaneService`
/// populates it on `deploy()`/clears it on `undeploy()`; `RouteHandlerInner`
/// holds the same `Arc` for lookup from
/// `crates/router/src/route_handler/http.rs`.
pub type HttpRouteRegistry = Arc<DashMap<String, Vec<HttpRoute>>>;

/// Matches a single `{param}` path pattern (e.g. `/orders/{id}`) against a
/// request path. Returns `None` if the pattern doesn't match at all,
/// `Some(None)` if it matches with no captured parameter, `Some(Some(v))` if
/// it matches and captured `v`. Only a single `{param}` segment is supported
/// (sufficient for every route shape `task.md` specifies) -- no general
/// globbing/regex.
///
/// Lives in `core`, not `router` (M06A A1, R3-A): `syneroym-control-plane`'s
/// deploy-time asset/route collision check (`D-A1-4`) needs it too, and it
/// must not depend on `syneroym-router` to get it.
pub fn match_path(pattern: &str, path: &str) -> Option<Option<String>> {
    let pattern_segs: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let path_segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if pattern_segs.len() != path_segs.len() {
        return None;
    }
    let mut captured = None;
    for (p, s) in pattern_segs.iter().zip(path_segs.iter()) {
        if p.starts_with('{') && p.ends_with('}') {
            captured = Some((*s).to_string());
        } else if p != s {
            return None;
        }
    }
    Some(captured)
}

/// The name of the single capturing segment in `pattern` (`/orders/{id}` ->
/// `Some("id")`), or `None` when the pattern has no `{...}` segment.
///
/// Returns the **last** such segment, matching `match_path`'s own last-wins
/// capture (M06A `D-A2-4`): with two `{...}` segments the two functions must
/// describe the same segment, or a guest would receive a name and a value
/// from different parts of the path. Only a single capture is supported
/// anyway.
#[must_use]
pub fn param_name(pattern: &str) -> Option<&str> {
    pattern
        .split('/')
        .filter(|s| !s.is_empty())
        .rfind(|s| s.starts_with('{') && s.ends_with('}'))
        .map(|s| &s[1..s.len() - 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_path_captures_a_single_param() {
        assert_eq!(match_path("/orders/{id}", "/orders/abc123"), Some(Some("abc123".to_string())));
    }

    #[test]
    fn match_path_matches_exact_literal_with_no_param() {
        assert_eq!(match_path("/orders", "/orders"), Some(None));
    }

    #[test]
    fn match_path_rejects_different_segment_counts() {
        assert_eq!(match_path("/orders", "/orders/abc123"), None);
        assert_eq!(match_path("/orders/{id}", "/orders"), None);
    }

    #[test]
    fn match_path_rejects_mismatched_literal_segments() {
        assert_eq!(match_path("/orders/{id}", "/events/abc123"), None);
    }

    #[test]
    fn param_name_returns_the_last_captured_segment() {
        assert_eq!(param_name("/orders/{id}"), Some("id"));
        assert_eq!(param_name("/a/{x}/b/{y}"), Some("y"));
    }

    #[test]
    fn param_name_returns_none_for_a_literal_pattern() {
        assert_eq!(param_name("/orders"), None);
    }

    #[test]
    fn param_name_agrees_with_match_path_on_a_two_capture_pattern() {
        let pattern = "/a/{x}/b/{y}";
        let path = "/a/one/b/two";
        assert_eq!(match_path(pattern, path), Some(Some("two".to_string())));
        assert_eq!(param_name(pattern), Some("y"));
    }
}
