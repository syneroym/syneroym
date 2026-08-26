//! Profile service application logic, target-independent.

use serde_json::json;
use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    envelope::{Request, Response},
    services,
};

/// This service's own schema version. Bumped by whichever slice changes
/// what this service stores; read by `status` and by nothing else in C2.
pub const SCHEMA_VERSION: u32 = 1;

pub async fn status<H: AppHost>(_host: &H) -> Result<String, String> {
    Ok(json!({
        "service": services::PROFILE.name,
        "schema_version": SCHEMA_VERSION,
    })
    .to_string())
}

pub async fn invoke<H: AppHost>(_host: &H, req: Request) -> Response {
    match req.method.as_str() {
        // C2 declares no product verbs, and reports no caller identity
        // (D-C2-4, F15): a sibling has no sound way to learn who is
        // asking, so nothing here pretends to. `ping` exists only so the
        // shared suite can prove, on both builds, that a request routed
        // through `web` reaches this service and a real answer comes
        // back -- reachability, not identity.
        "profile.ping" => Response::ok(json!({ "service": services::PROFILE.name })),
        other => Response::method_not_found(other),
    }
}
