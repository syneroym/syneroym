//! The shared per-service HTTP route vocabulary (M3B Slice 7): `HttpRoute`
//! and the `HttpRouteRegistry` table it's kept in.
//!
//! Lives in `core` rather than in `router` or `control_plane` because both
//! sides need it without depending on each other: `syneroym-control-plane`
//! parses these out of a deployed service's `custom_config` on deploy/
//! undeploy (`ControlPlaneService`, `crates/control_plane/src/http_routes.rs`),
//! and `syneroym-router` reads the registry per HTTP request
//! (`crates/router/src/route_handler/http.rs`) to decide how a given verb+
//! path bridges onto `data-layer`/`messaging`/a registered stream protocol.

use std::sync::Arc;

use dashmap::DashMap;
use serde::Deserialize;

/// One `http_routes` entry. `target` selects which native capability the
/// route bridges onto; the optional fields are only meaningful for the
/// matching target (`collection` for `data-layer`, `topic` for `messaging`,
/// `protocol` for `stream`) and are ignored otherwise.
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
}

/// Shared, keyed-by-`service_id` HTTP route table. `ControlPlaneService`
/// populates it on `deploy()`/clears it on `undeploy()`; `RouteHandlerInner`
/// holds the same `Arc` for lookup from
/// `crates/router/src/route_handler/http.rs`.
pub type HttpRouteRegistry = Arc<DashMap<String, Vec<HttpRoute>>>;
