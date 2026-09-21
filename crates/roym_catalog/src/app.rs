//! Catalog service application logic, target-independent.
//!
//! The provider's offer: a signed `listing` record with a stable
//! content-derived id, edited only by producing a new version that
//! `supersedes` the last, plus unsigned availability state and the
//! catalog-side publication limiter.

use std::{cmp::Reverse, collections::BTreeMap};

use serde::Deserialize;
use serde_json::{Map, Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost, AppSigning,
    types::{
        data_layer::{
            CollectionSchema, IndexDefinition, IndexType, Mutation, QueryOptions, RecordWriteValue,
        },
        proxy::CallTarget,
        signing::{Principal, RecordDraft},
    },
};
use syneroym_roym_core::{
    admit,
    backup::{BUNDLE_VERSION, Bundle, BundleManifest, SECTION_AVAILABILITY, SECTION_LISTINGS},
    clock,
    envelope::{Request, Response},
    listing::{self, ListingPayload, ListingStatus},
    person::ProfilePayload,
    record::{Envelope, RECORD_LISTING, VerifyOptions, content_digest, verify_json},
    safety::{self, Admission, PublicationLimits},
    services,
    signing::{self, CertificateError},
};

/// Bumped in this slice: the service gains its first state.
pub const SCHEMA_VERSION: u32 = 2;

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
    if let Some(resp) = signing::handle_certificate_verb(host, "catalog.", &req).await {
        return resp;
    }

    match req.method.as_str() {
        "listing.ping" => Response::ok(json!({ "service": services::CATALOG.name })),
        "listing.set" => listing_ops::set_listing(host, &req).await,
        "listing.withdraw" => listing_ops::withdraw_listing(host, &req).await,
        "listing.get" => listing_ops::get_listing(host, &req).await,
        "listing.list" => listing_ops::list_listings(host, &req).await,
        "listing.history" => listing_ops::listing_history(host, &req).await,
        "listing.verify" => listing_ops::verify_listing(host, &req).await,
        "listing.limits" => match limits::load_publication_limits(host).await {
            Ok(l) => Response::ok(json!(l)),
            Err(e) => Response::internal_error(e),
        },
        "listing.set-limits" => limits::set_limits(host, &req).await,
        "availability.set" => availability::availability_set(host, &req).await,
        "availability.list" => availability::availability_list(host, &req).await,
        "availability.remove" => availability::availability_remove(host, &req).await,
        "catalog.export" => backup::export(host).await,
        "catalog.import" => backup::import(host, &req).await,
        other => Response::method_not_found(other),
    }
}

mod availability;
mod backup;
mod limits;
mod listing_ops;
