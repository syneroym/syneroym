//! Transaction service application logic, target-independent.

use serde_json::json;
use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    envelope::{Request, Response},
    services,
};

pub const SCHEMA_VERSION: u32 = 1;

pub async fn status<H: AppHost>(_host: &H) -> Result<String, String> {
    Ok(json!({
        "service": services::TRANSACTION.name,
        "schema_version": SCHEMA_VERSION,
    })
    .to_string())
}

pub async fn invoke<H: AppHost>(_host: &H, req: Request) -> Response {
    match req.method.as_str() {
        "request.ping" | "quote.ping" | "agreement.ping" | "receipt.ping" => {
            Response::ok(json!({ "service": services::TRANSACTION.name }))
        }
        other => Response::method_not_found(other),
    }
}
