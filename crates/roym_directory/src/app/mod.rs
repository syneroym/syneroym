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
pub mod client_query;
pub mod client_sources;
pub mod publication_ops;
pub mod search_ops;
pub mod synorg;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{
        CollectionSchema, IndexDefinition, IndexType, QueryOptions, RecordWriteValue,
    },
};
use syneroym_roym_core::{
    admit::{self, WireRule},
    directory::DIRECTORY_SCHEMA_VERSION,
    envelope::{Request, Response},
    services,
};

pub const SCHEMA_VERSION: u32 = DIRECTORY_SCHEMA_VERSION;

pub const SETTINGS: &str = "settings";
pub const MEMBERS: &str = "members";
pub const PUBLICATIONS: &str = "publications";
pub const SEARCH_INDEX: &str = "search_index";
pub const PUBLICATION_LOG: &str = "publication_log";
pub const SOURCES: &str = "sources";
pub const SEARCH_RUNS: &str = "search_runs";
pub const RUNS: &str = "runs";
/// Transient per-node bookkeeping (the retention-prune rate marker).
/// Deliberately its own collection: it must never ride into an
/// `export` bundle, where a stale or future `at_secs` from another node
/// would skew the importing node's prune schedule.
pub const NODE_STATE: &str = "node_state";

pub(crate) const SETTINGS_KEY: &str = "synorg";

/// The three methods a foreign node may reach. `directory.search` and
/// `directory.info` admit a stranger with no identity at all -- reading
/// something this installation publishes on purpose costs nothing to
/// leave open. `directory.publish` records who asked, because a
/// publication is durable and must name a party.
const WIRE_REACHABLE: &[(&str, WireRule)] = &[
    ("directory.search", WireRule::Open),
    ("directory.info", WireRule::Open),
    ("directory.publish", WireRule::VerifiedOnly),
];

pub async fn status<H: AppHost>(_host: &H) -> Result<String, String> {
    Ok(json!({
        "service": services::DIRECTORY.name,
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

/// `search_runs` rows are per-run working state. `run_id` is the filter
/// `merge` and `run-envelope` narrow on; `at_secs` is the field
/// `start-run` prunes by. Both halves of the service that touch this
/// collection create it with the same indexes.
pub(crate) fn search_runs_indexes() -> [IndexDefinition; 2] {
    [idx("run_id", IndexType::String), idx("at_secs", IndexType::Numeric)]
}

/// Every row of `collection`, oldest write order, paging until the
/// data-layer's own cursor answers `None`.
pub(crate) async fn collect_raw<H: AppHost>(
    host: &H,
    collection: &str,
) -> Result<Vec<(String, Value)>, String> {
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let page = AppDataLayer::query(
            host,
            collection.to_string(),
            QueryOptions { filter: None, limit: Some(500), cursor: cursor.clone() },
        )
        .await
        .map_err(|e| e.to_string())?;
        for r in page.records {
            if let Ok(parsed) = serde_json::from_slice::<Value>(&r.payload) {
                out.push((r.id, parsed));
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(out)
}

/// Every row of `collection` whose stored JSON matches `filter`, paging
/// the data-layer cursor. The filter runs at the host, so a large
/// collection is never materialized whole in guest memory.
pub(crate) async fn collect_raw_where<H: AppHost>(
    host: &H,
    collection: &str,
    filter: &Value,
) -> Result<Vec<(String, Value)>, String> {
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let page = AppDataLayer::query(
            host,
            collection.to_string(),
            QueryOptions {
                filter: Some(filter.to_string()),
                limit: Some(500),
                cursor: cursor.clone(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        for r in page.records {
            if let Ok(parsed) = serde_json::from_slice::<Value>(&r.payload) {
                out.push((r.id, parsed));
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(out)
}

pub(crate) async fn collect<H: AppHost>(host: &H, collection: &str) -> Result<Vec<Value>, String> {
    Ok(collect_raw(host, collection)
        .await?
        .into_iter()
        .map(|(id, payload)| json!({ "id": id, "payload": payload }))
        .collect())
}

pub(crate) async fn put_json<H: AppHost>(
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

pub(crate) async fn get_json<H: AppHost, T: for<'de> Deserialize<'de>>(
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

pub async fn invoke<H: AppHost>(host: &H, req: Request) -> Response {
    let caller = match admit::admit(host, WIRE_REACHABLE, &req.method).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    match req.method.as_str() {
        "directory.ping" => Response::ok(json!({ "service": services::DIRECTORY.name })),
        // Server half.
        "directory.settings" => synorg::get_settings(host).await,
        "directory.set-settings" => synorg::set_settings(host, &req).await,
        "directory.info" => synorg::info(host).await,
        "member.add" => synorg::member_add(host, &req).await,
        "member.remove" => synorg::member_remove(host, &req).await,
        "member.list" => synorg::member_list(host).await,
        "directory.publish" => publication_ops::publish(host, &req, caller).await,
        "directory.unpublish" => publication_ops::unpublish(host, &req).await,
        "directory.publications" => publication_ops::publications(host).await,
        "directory.search" => search_ops::search(host, &req).await,
        "directory.limits" => match publication_ops::load_publication_limits(host).await {
            Ok(l) => Response::ok(json!(l)),
            Err(e) => Response::internal_error(e),
        },
        "directory.set-limits" => publication_ops::set_limits(host, &req).await,
        "directory.reindex" => search_ops::reindex(host).await,
        "directory.export" => backup::export(host).await,
        "directory.import" => backup::import(host, &req).await,
        // Client half.
        "directory.add-source" => client_sources::add_source(host, &req).await,
        "directory.probe-info" => client_sources::probe_info_verb(host, &req).await,
        "directory.remove-source" => client_sources::remove_source(host, &req).await,
        "directory.sources" => client_sources::sources(host).await,
        "directory.start-run" => client_query::start_run(host).await,
        "directory.query-source" => client_query::query_source(host, &req).await,
        "directory.merge" => client_query::merge(host, &req).await,
        "directory.run-envelope" => client_query::run_envelope(host, &req).await,
        "directory.publish-to-source" => client_sources::publish_to_source(host, &req).await,
        other => Response::method_not_found(other),
    }
}
