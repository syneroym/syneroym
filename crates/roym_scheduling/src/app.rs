//! Scheduling service application logic, target-independent.
//!
//! Two kinds of state: the owner's availability slots, and the reservations
//! that the deal core takes on them through the reserve hooks.

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

pub(crate) mod availability;
pub(crate) mod backup;
pub(crate) mod prepare;
pub(crate) mod reservation;
#[cfg(test)]
mod tests;

/// This service's own schema version. Bumped by whichever change alters what
/// this service stores; read by `status` and by nothing else.
pub const SCHEMA_VERSION: u32 = 1;

pub const AVAILABILITY: &str = "availability";
pub const RESERVATIONS: &str = "reservations";

pub async fn status<H: AppHost>(_host: &H) -> Result<String, String> {
    Ok(json!({
        "service": services::SCHEDULING.name,
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
        CollectionSchema { name: name.to_string(), indexes: indexes.to_vec(), text_fields: vec![] },
    )
    .await
    .map_err(|e| e.to_string())
}

pub(crate) fn idx(field: &str, ty: IndexType) -> IndexDefinition {
    IndexDefinition { field_name: field.to_string(), type_: ty }
}

pub(crate) async fn ensure_availability<H: AppHost>(host: &H) -> Result<(), String> {
    ensure_coll(
        host,
        AVAILABILITY,
        &[idx("listing_id", IndexType::String), idx("start_secs", IndexType::Numeric)],
    )
    .await
}

pub(crate) async fn ensure_reservations<H: AppHost>(host: &H) -> Result<(), String> {
    ensure_coll(
        host,
        RESERVATIONS,
        &[idx("kind", IndexType::String), idx("slot_id", IndexType::String)],
    )
    .await
}

/// A refusal the caller can act on: `invalid_params` with the code in
/// `data.code`. The message is the code.
pub(crate) fn refuse(code: &str) -> Response {
    Response::invalid_params(code).with_data(json!({ "code": code }))
}

pub async fn invoke<H: AppHost>(host: &H, req: Request) -> Response {
    if let Some(resp) = admit::require_internal(host).await {
        return resp;
    }
    api::dispatch(host, &req).await.unwrap_or_else(|| Response::method_not_found(&req.method))
}
