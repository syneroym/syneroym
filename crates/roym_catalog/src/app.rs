//! Catalog service application logic, target-independent.
//!
//! The listing owner's offer: a signed `listing` record with a stable
//! content-derived id, edited only by producing a new version that
//! `supersedes` the last, plus unsigned availability state and the
//! catalog-side publication limiter.

use serde_json::json;
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{CollectionSchema, IndexDefinition, IndexType},
};
use syneroym_roym_core::{
    admit,
    envelope::{Request, Response},
    services,
};

use crate::api;

/// Version 2 gave the service its first state. Version 3 renames the
/// service-location values inside signed listings.
pub const SCHEMA_VERSION: u32 = 3;

pub const LISTINGS: &str = "listings";
pub const LISTING_HISTORY: &str = "listing_history";
pub const AVAILABILITY: &str = "availability";
pub const PUBLICATIONS: &str = "publications";
pub const SETTINGS: &str = "settings";
pub const PUBLICATION_LIMITS_KEY: &str = "publication_limits";

pub async fn status<H: AppHost>(_host: &H) -> Result<String, String> {
    Ok(json!({
        "service": services::CATALOG.name,
        "schema_version": SCHEMA_VERSION,
    })
    .to_string())
}

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

pub(crate) fn idx(field: &str, ty: IndexType) -> IndexDefinition {
    IndexDefinition { field_name: field.to_string(), type_: ty }
}

pub(crate) async fn ensure_listings<H: AppHost>(host: &H) -> Result<(), String> {
    ensure_coll(
        host,
        LISTINGS,
        &[idx("status", IndexType::String), idx("updated_at_secs", IndexType::Numeric)],
    )
    .await
}

pub(crate) async fn ensure_availability<H: AppHost>(host: &H) -> Result<(), String> {
    ensure_coll(
        host,
        AVAILABILITY,
        &[idx("listing_id", IndexType::String), idx("start_secs", IndexType::Numeric)],
    )
    .await
}

pub async fn invoke<H: AppHost>(host: &H, req: Request) -> Response {
    if let Some(resp) = admit::require_internal(host).await {
        return resp;
    }
    if let Some(resp) = api::dispatch(host, &req).await {
        return resp;
    }
    Response::method_not_found(&req.method)
}

pub(crate) mod availability;
pub(crate) mod backup;
pub(crate) mod limits;
pub(crate) mod listing_ops;
