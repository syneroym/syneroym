//! Directory service application logic, target-independent.
//!
//! One service, two halves, never sharing a collection: the **server**
//! half (a SynOrg's own settings, roster, publications and search) and the
//! **client** half (this person's own list of directories, the fan-out,
//! and the merge). The server half admits a stranger's signed bytes; the
//! client half never talks to a stranger directly -- it asks the node to
//! make one bounded call per directory and reads back what the node
//! verified.

pub mod backup;
pub mod client_merge;
pub mod client_query;
pub mod client_sources;
pub mod credential_ops;
pub mod held;
pub mod moderation_ops;
pub mod publication_ops;
pub mod search_ops;
pub mod standing;
pub mod synorg;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost, AppSigning,
    types::{
        data_layer::{CollectionSchema, IndexDefinition, IndexType, RecordWriteValue},
        signing::RecordDraft,
    },
};
use syneroym_roym_core::{
    admit,
    area::Area,
    clock,
    directory::{self, DIRECTORY_SCHEMA_VERSION},
    envelope::{Request, Response},
    membership::{self, CheckInput, ListingRef, MembershipVerdict},
    paging, person,
    record::{self, Envelope},
    services,
    signing::{self, CertificateError},
};

use crate::api;

pub const SCHEMA_VERSION: u32 = DIRECTORY_SCHEMA_VERSION;

pub const SETTINGS: &str = "settings";
pub const MEMBERS: &str = "members";
pub const PUBLICATIONS: &str = "publications";
pub const SEARCH_INDEX: &str = "search_index";
pub const PUBLICATION_LOG: &str = "publication_log";
pub const SOURCES: &str = "sources";
pub const SEARCH_RUNS: &str = "search_runs";
pub const RUNS: &str = "runs";
pub const CREDENTIALS: &str = "credentials";
pub const REVOCATIONS: &str = "revocations";
pub const DECISIONS: &str = "moderation_decisions";
pub const STANDING: &str = "standing";
pub const HELD_MEMBERSHIPS: &str = "held_memberships";
/// Transient per-node bookkeeping (the retention-prune rate marker).
/// Deliberately its own collection: it must never ride into an
/// `export` bundle, where a stale or future `at_secs` from another node
/// would skew the importing node's prune schedule.
pub const NODE_STATE: &str = "node_state";

pub(in crate::app) const SETTINGS_KEY: &str = "synorg";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DidInput {
    pub did: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemberFilterInput {
    pub member_did: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RemovedOutput {
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SignedRecordOutput {
    pub record_id: String,
    pub envelope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct IssuedRecordsOutput {
    pub records: Vec<standing::IssuedRecordRow>,
}

pub async fn status<H: AppHost>(_host: &H) -> Result<String, String> {
    Ok(json!({
        "service": services::DIRECTORY.name,
        "schema_version": SCHEMA_VERSION,
    })
    .to_string())
}

pub(in crate::app) async fn ensure_coll<H: AppHost>(
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

pub(in crate::app) fn idx(field: &str, ty: IndexType) -> IndexDefinition {
    IndexDefinition { field_name: field.to_string(), type_: ty }
}

/// `search_runs` rows are per-run working state. `run_id` is the filter
/// `merge` and `run-envelope` narrow on; `at_secs` is the field
/// `start-run` prunes by. Both halves of the service that touch this
/// collection create it with the same indexes.
pub(in crate::app) fn search_runs_indexes() -> [IndexDefinition; 2] {
    [idx("run_id", IndexType::String), idx("at_secs", IndexType::Numeric)]
}

/// `SEARCH_INDEX` is created with these everywhere it is created (a
/// literal list here would let one call site's indexes drift from
/// another's). `issuer` backs `rewrite_listed_windows`'s per-member
/// query, `listing_id`/`status` back the plain listing lookups, and
/// `listed_until_secs` backs the host-side listed-window filter.
pub(in crate::app) fn search_index_indexes() -> [IndexDefinition; 4] {
    [
        idx("listing_id", IndexType::String),
        idx("status", IndexType::String),
        idx("issuer", IndexType::String),
        idx("listed_until_secs", IndexType::Numeric),
    ]
}

pub(in crate::app) async fn put_json<H: AppHost>(
    host: &H,
    collection: &str,
    id: &str,
    value: &impl Serialize,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    AppDataLayer::put(
        host,
        collection.to_string(),
        RecordWriteValue { id: id.to_string(), payload: bytes },
    )
    .await
    .map_err(|e| e.to_string())
}

pub(in crate::app) async fn get_json<H: AppHost, T: for<'de> Deserialize<'de>>(
    host: &H,
    collection: &str,
    id: &str,
) -> Result<Option<T>, String> {
    let row = AppDataLayer::get(host, collection.to_string(), id.to_string())
        .await
        .map_err(|e| e.to_string())?;
    match row {
        Some(r) => serde_json::from_slice(&r.payload).map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

pub(in crate::app) async fn owner_did_or_node<H: AppHost>(host: &H) -> String {
    match AppSigning::signing_identity(host).await {
        Ok(id) => id.owner_did.unwrap_or(id.signing_did),
        Err(_) => String::new(),
    }
}

/// The DID this SynOrg's signed statements are issued under: the
/// installation's recorded owner. `None` when there is no owner -- such a
/// node can run a directory but can issue nothing.
pub(in crate::app) async fn issuer_did<H: AppHost>(host: &H) -> Option<String> {
    signing::owner_did(host).await.ok()
}

/// A value's own serde wire spelling (e.g. `past-counterparties`, not
/// `Debug`'s `PastCounterparties`) -- the shape every enum here declares
/// with `#[serde(rename_all = "kebab-case")]`, and the shape a caller
/// filters on. `{:?}` and `.to_lowercase()` agree only for single-word
/// variants; a multi-word one indexes under a string nothing else in the
/// product ever produces, and a query for the documented value silently
/// matches nothing.
pub(in crate::app) fn serde_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|j| j.as_str().map(str::to_string)).unwrap_or_default()
}

pub async fn invoke<H: AppHost>(host: &H, req: Request) -> Response {
    if let Err(resp) = admit::admit(host, api::WIRE_REACHABLE, &req.method).await {
        return resp;
    }

    if let Some(resp) = api::dispatch(host, &req).await {
        return resp;
    }

    Response::method_not_found(&req.method)
}
