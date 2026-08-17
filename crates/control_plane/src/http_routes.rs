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
//! stream protocol, or (M06A A2) reaches the deployed component's own
//! `syneroym:http/incoming-handler#handle-request` export.

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
        ("guest", "handle-request") => Ok(()),
        ("guest", other) => Err(format!(
            "http_routes entry `{} {}` has target=guest with unsupported operation `{other}`; the \
             only guest operation is `handle-request`",
            route.method, route.path
        )),
        ("websocket", "handle-upgrade") => Ok(()),
        ("websocket", other) => Err(format!(
            "http_routes entry `{} {}` has target=websocket with unsupported operation `{other}`; \
             the only websocket operation is `handle-upgrade`",
            route.method, route.path
        )),
        _ => Ok(()),
    }?;

    // M06A D-A2-7, A3, A4: `public` does nothing outside a guest or websocket
    // route, so accepting it there would be exactly the silently-dead
    // configuration this module's duplicate-route check already exists to
    // prevent.
    if route.public && (route.target != "guest" && route.target != "websocket") {
        return Err(format!(
            "http_routes entry `{} {}` sets `public` on target={}; `public` is only meaningful \
             for target=guest or target=websocket",
            route.method, route.path, route.target
        ));
    }
    Ok(())
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
                {"method": "GET", "path": "/upload", "target": "stream", "operation": "accept-upload", "protocol": "test"}
            ]
        });
        assert!(parse_http_routes(&json).unwrap_err().contains("PUT or POST"));
    }

    #[test]
    fn websocket_target_valid() {
        let json = serde_json::json!({
            "http_routes": [
                {"method": "GET", "path": "/ws", "target": "websocket", "operation": "handle-upgrade"}
            ]
        });
        let routes = parse_http_routes(&json).unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].target, "websocket");
        assert_eq!(routes[0].operation, "handle-upgrade");
    }

    #[test]
    fn websocket_target_invalid_operation() {
        let json = serde_json::json!({
            "http_routes": [
                {"method": "GET", "path": "/ws", "target": "websocket", "operation": "invalid-op"}
            ]
        });
        assert!(
            parse_http_routes(&json)
                .unwrap_err()
                .contains("the only websocket operation is `handle-upgrade`")
        );
    }

    #[test]
    fn public_true_allowed_for_websocket() {
        let json = serde_json::json!({
            "http_routes": [
                {"method": "GET", "path": "/ws", "target": "websocket", "operation": "handle-upgrade", "public": true}
            ]
        });
        let routes = parse_http_routes(&json).unwrap();
        assert!(routes[0].public);
    }

    #[test]
    fn guest_route_with_handle_request_operation_is_accepted() {
        let json = serde_json::json!({
            "http_routes": [
                {"method": "GET", "path": "/echo", "target": "guest", "operation": "handle-request"}
            ]
        });
        assert!(parse_http_routes(&json).is_ok());
    }

    #[test]
    fn guest_route_with_unsupported_operation_is_rejected() {
        let json = serde_json::json!({
            "http_routes": [
                {"method": "GET", "path": "/echo", "target": "guest", "operation": "get"}
            ]
        });
        assert!(parse_http_routes(&json).unwrap_err().contains("handle-request"));
    }

    #[test]
    fn public_true_on_a_non_guest_or_websocket_target_is_rejected() {
        let json = serde_json::json!({
            "http_routes": [
                {"method": "GET", "path": "/orders/{id}", "target": "data-layer",
                 "operation": "get", "collection": "orders", "public": true}
            ]
        });
        assert!(parse_http_routes(&json).unwrap_err().contains("public"));
    }

    #[test]
    fn public_true_on_a_guest_target_is_accepted() {
        let json = serde_json::json!({
            "http_routes": [
                {"method": "GET", "path": "/echo", "target": "guest",
                 "operation": "handle-request", "public": true}
            ]
        });
        assert!(parse_http_routes(&json).is_ok());
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
