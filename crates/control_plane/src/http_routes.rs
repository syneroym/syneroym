//! Parses a deployed service's declared HTTP routes (M3B Slice 7) out of the
//! `http_routes` key of its `custom_config` JSON
//! (`ServiceConfig.custom_config`, already a free-form per-service deploy-time
//! extension point -- see `ControlPlaneService::deploy`). The
//! `HttpRoute`/`HttpRouteRegistry` types this module produces live in
//! `syneroym_core::http_routes`, shared with `syneroym-router` (see that
//! module's doc comment for why).
//!
//! `task.md` (`§B8`) requires HTTP routes to be per-service, not a global
//! substrate-wide policy, since different services expose different
//! data-layer collections / messaging topics. Reusing `custom_config`
//! avoids adding new WIT surface for this: `crates/router/src/route_handler/
//! http.rs` looks routes up by `service_id` at request time to decide how a
//! given HTTP verb+path bridges onto `data-layer`/`messaging`/a registered
//! stream protocol.

use serde::Deserialize;
use syneroym_core::http_routes::HttpRoute;

#[derive(Debug, Default, Deserialize)]
struct HttpRoutesConfig {
    #[serde(default)]
    http_routes: Vec<HttpRoute>,
}

/// Parses the `http_routes` array out of a deployed service's already-parsed
/// `custom_config` JSON. Absent key => no routes (`Ok(vec![])`), not an
/// error -- most services declare no HTTP routes at all. A present but
/// malformed `http_routes` value is a deploy-time configuration error, same
/// severity as the existing JSON-schema validation step -- this includes a
/// route missing the field its `target`/`operation` combination requires
/// (`collection`/`topic`/`protocol`), which previously fell back silently
/// to an empty string at request time (`unwrap_or_default()` in
/// `crates/router/src/route_handler/http.rs`) instead of failing here where
/// the misconfiguration actually happened.
pub fn parse_http_routes(custom_json: &serde_json::Value) -> Result<Vec<HttpRoute>, String> {
    let config: HttpRoutesConfig = serde_json::from_value(custom_json.clone())
        .map_err(|e| format!("invalid http_routes in custom_config: {e}"))?;
    for route in &config.http_routes {
        validate_route(route)?;
    }
    reject_duplicate_routes(&config.http_routes)?;
    Ok(config.http_routes)
}

/// Checks the one field each `target`/`operation` combination actually
/// reads is present and non-empty, and (for `stream`/`accept-upload`) that
/// the declared HTTP method can plausibly carry a request body -- a `GET`
/// route wired to `accept-upload` would otherwise attempt to read an
/// upload stream from a body-less request.
fn validate_route(route: &HttpRoute) -> Result<(), String> {
    let field_required = |field: &str, value: &Option<String>| -> Result<(), String> {
        if value.as_deref().unwrap_or_default().is_empty() {
            Err(format!(
                "http_routes entry `{} {}` (target={}, operation={}) requires a non-empty \
                 `{field}`",
                route.method, route.path, route.target, route.operation
            ))
        } else {
            Ok(())
        }
    };
    match (route.target.as_str(), route.operation.as_str()) {
        ("data-layer", "get" | "query" | "put" | "patch") => {
            field_required("collection", &route.collection)
        }
        ("messaging", "publish" | "subscribe-sse") => field_required("topic", &route.topic),
        ("stream", "accept-upload") => {
            field_required("protocol", &route.protocol)?;
            if !route.method.eq_ignore_ascii_case("PUT")
                && !route.method.eq_ignore_ascii_case("POST")
            {
                return Err(format!(
                    "http_routes entry `{} {}` (target=stream, operation=accept-upload) must use \
                     PUT or POST, not {}",
                    route.method, route.path, route.method
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Rejects a `Vec<HttpRoute>` containing two entries for the same
/// (method, path) pair -- `resolve_route`'s `find_map` would otherwise
/// silently pick the first and make the second permanently dead
/// configuration, with no warning at deploy time or request time.
fn reject_duplicate_routes(routes: &[HttpRoute]) -> Result<(), String> {
    for (i, a) in routes.iter().enumerate() {
        for b in &routes[i + 1..] {
            if a.method.eq_ignore_ascii_case(&b.method) && a.path == b.path {
                return Err(format!(
                    "duplicate http_routes entry for {} {} -- the second entry would be dead \
                     configuration, silently shadowed by the first",
                    a.method, a.path
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_routes_from_custom_config() {
        let json = serde_json::json!({
            "http_routes": [
                {"method": "GET", "path": "/orders/{id}", "target": "data-layer",
                 "operation": "get", "collection": "orders"},
                {"method": "POST", "path": "/events", "target": "messaging",
                 "operation": "publish", "topic": "events"},
                {"method": "PUT", "path": "/upload", "target": "stream",
                 "operation": "accept-upload", "protocol": "file-transfer"},
            ]
        });
        let routes = parse_http_routes(&json).unwrap();
        assert_eq!(routes.len(), 3);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].collection.as_deref(), Some("orders"));
        assert_eq!(routes[1].topic.as_deref(), Some("events"));
        assert_eq!(routes[2].protocol.as_deref(), Some("file-transfer"));
    }

    #[test]
    fn absent_http_routes_key_is_empty_not_an_error() {
        let json = serde_json::json!({"some_other_key": "value"});
        let routes = parse_http_routes(&json).unwrap();
        assert!(routes.is_empty());
    }

    #[test]
    fn malformed_http_routes_is_an_error() {
        let json = serde_json::json!({"http_routes": [{"method": "GET"}]});
        assert!(parse_http_routes(&json).is_err());
    }

    #[test]
    fn data_layer_route_without_collection_is_rejected() {
        let json = serde_json::json!({
            "http_routes": [
                {"method": "GET", "path": "/orders/{id}", "target": "data-layer", "operation": "get"}
            ]
        });
        assert!(parse_http_routes(&json).unwrap_err().contains("collection"));
    }

    #[test]
    fn messaging_route_without_topic_is_rejected() {
        let json = serde_json::json!({
            "http_routes": [
                {"method": "POST", "path": "/events", "target": "messaging", "operation": "publish"}
            ]
        });
        assert!(parse_http_routes(&json).unwrap_err().contains("topic"));
    }

    #[test]
    fn stream_upload_route_without_protocol_is_rejected() {
        let json = serde_json::json!({
            "http_routes": [
                {"method": "PUT", "path": "/upload", "target": "stream", "operation": "accept-upload"}
            ]
        });
        assert!(parse_http_routes(&json).unwrap_err().contains("protocol"));
    }

    #[test]
    fn stream_upload_route_with_get_method_is_rejected() {
        let json = serde_json::json!({
            "http_routes": [
                {"method": "GET", "path": "/upload", "target": "stream",
                 "operation": "accept-upload", "protocol": "file-transfer"}
            ]
        });
        assert!(parse_http_routes(&json).unwrap_err().contains("PUT or POST"));
    }

    #[test]
    fn duplicate_method_and_path_is_rejected() {
        let json = serde_json::json!({
            "http_routes": [
                {"method": "GET", "path": "/orders/{id}", "target": "data-layer",
                 "operation": "get", "collection": "orders"},
                {"method": "get", "path": "/orders/{id}", "target": "data-layer",
                 "operation": "get", "collection": "archived-orders"},
            ]
        });
        assert!(parse_http_routes(&json).unwrap_err().contains("duplicate"));
    }
}
