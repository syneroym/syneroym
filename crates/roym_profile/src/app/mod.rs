//! Profile service application logic, target-independent.

pub mod backup;
pub mod contacts;
pub mod moderation;
pub mod profile_ops;

pub use contacts::ContactRow;
pub use moderation::{BlockRow, ReportRow};
use serde_json::json;
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{CollectionSchema, IndexDefinition},
};
use syneroym_roym_core::{
    admit,
    envelope::{Request, Response},
    services, signing,
};

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

    if let Some(resp) = signing::handle_certificate_verb(host, "profile.", &req).await {
        return resp;
    }

    match req.method.as_str() {
        "profile.ping" => Response::ok(json!({ "service": services::PROFILE.name })),
        "profile.policy" => profile_ops::policy(),
        "profile.get" => profile_ops::get(host, &req).await,
        "profile.set" => profile_ops::set(host, &req).await,
        "profile.export" => backup::export(host).await,
        "profile.import" => backup::import(host, &req).await,
        "contacts.list" => contacts::list(host, &req).await,
        "contacts.get" => contacts::get(host, &req).await,
        "contacts.upsert" => contacts::upsert(host, &req).await,
        "contacts.remove" => contacts::remove(host, &req).await,
        "contacts.resolve-address" => contacts::resolve_address(host, &req).await,
        "contacts.admit-first-contact" => contacts::admit_first_contact(host, &req).await,
        "contacts.limits" => contacts::limits(host).await,
        "contacts.set-limits" => contacts::set_limits(host, &req).await,
        "block.add" => moderation::block_add(host, &req).await,
        "block.remove" => moderation::block_remove(host, &req).await,
        "block.list" => moderation::block_list(host, &req).await,
        "block.check" => moderation::block_check(host, &req).await,
        "report.create" => moderation::report_create(host, &req).await,
        "report.list" => moderation::report_list(host, &req).await,
        "report.get" => moderation::report_get(host, &req).await,
        "report.withdraw" => moderation::report_withdraw(host, &req).await,
        other => Response::method_not_found(other),
    }
}
