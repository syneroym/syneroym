//! Shared dual-build helpers for Roym services.

use std::future::Future;

use serde_json::Value;
use syneroym_app_host::AppHost;

use crate::envelope::{Request, Response};

/// Parses an `invoke` request string, dispatches it through the app's own
/// handler, and encodes the response. The `Err` arm is reserved for a
/// request that could not be parsed at all.
pub async fn handle_invoke<'a, H, F, Fut>(
    _host: &'a H,
    request: &str,
    f: F,
) -> Result<String, String>
where
    H: AppHost,
    F: FnOnce(&'a H, Request) -> Fut,
    Fut: Future<Output = Response>,
{
    let req: Request = match serde_json::from_str(request) {
        Ok(r) => r,
        Err(e) => return Err(format!("Invalid request JSON: {e}")),
    };
    let resp = f(_host, req).await;
    serde_json::to_string(&resp).map_err(|e| format!("Serialization error: {e}"))
}

/// The same two parameter shapes `json_to_wasm_params` accepts on the WASM
/// side -- positional `["<json>"]` or named `{"request": "<json>"}` -- so
/// one client frame drives both builds. Lifted verbatim from the fixture's
/// `extract_request_param`.
pub fn extract_request_param(params: &Value) -> Option<String> {
    match params {
        Value::Array(items) => items.first()?.as_str().map(str::to_string),
        Value::Object(map) => map.get("request")?.as_str().map(str::to_string),
        _ => None,
    }
}
