//! Profile service application logic, target-independent.

pub mod backup;
pub mod contacts;
pub mod moderation;
pub mod profile_ops;

pub use moderation::{BlockRow, ReportRow};
use serde_json::json;
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{CollectionSchema, IndexDefinition},
};
pub use syneroym_roym_core::method::profile::ContactRow;
use syneroym_roym_core::{
    admit,
    envelope::{Request, Response},
    services,
};

use crate::api;

/// This service's own schema version. Bumped by whichever slice changes
/// what this service stores; read by `status` and by nothing else.
pub const SCHEMA_VERSION: u32 = 2;

pub const PROFILES: &str = "profiles";
pub const PROFILE_HISTORY: &str = "profile_history";
pub const CONTACTS: &str = "contacts";
pub const BLOCKS: &str = "blocks";
pub const REPORTS: &str = "reports";
pub const CONTACT_ATTEMPTS: &str = "contact_attempts";
pub const SETTINGS: &str = "settings";

pub(crate) async fn ensure_coll<H: AppHost>(
    host: &H,
    name: &str,
    indexes: &[IndexDefinition],
) -> Result<(), String> {
    AppDataLayer::create_collection(
        host,
        CollectionSchema { name: name.to_string(), indexes: indexes.to_vec() },
    )
    .await
    .map_err(|e| e.to_string())
}

pub async fn status<H: AppHost>(_host: &H) -> Result<String, String> {
    Ok(json!({
        "service": services::PROFILE.name,
        "schema_version": SCHEMA_VERSION,
    })
    .to_string())
}

pub async fn invoke<H: AppHost>(host: &H, req: Request) -> Response {
    if let Some(resp) = admit::require_internal(host).await {
        return resp;
    }

    api::dispatch(host, &req).await.unwrap_or_else(|| Response::method_not_found(&req.method))
}
