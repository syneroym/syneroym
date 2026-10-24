//! The HTTP side of curated lists: one generic route per list, with
//! forwarding to the parent registry when this registry lists nothing.

use std::sync::Arc;

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::get,
};
use serde::Deserialize;
use syneroym_core::{
    config::roles::ServiceRegistryRole,
    curated_list::{self, CuratedList, ListEntry},
    registry_http,
};
use tracing::{debug, info, warn};

struct ListState<E> {
    entries: Vec<E>,
    max_age_secs: u64,
    parent_registry_url: Option<String>,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct HopQuery {
    #[serde(default)]
    hops: u8,
}

/// The routes of every curated list of this registry. A new list adds one
/// `.merge(list_route(...)?)` line here.
///
/// # Errors
/// When an entry is bad or a duplicate, or when `parent_registry_url` is
/// invalid. The registry then does not start.
pub(crate) fn router(role: &ServiceRegistryRole) -> anyhow::Result<Router> {
    let client = reqwest::Client::builder()
        .timeout(registry_http::HTTP_REQUEST_TIMEOUT)
        .build()
        .context("building the curated-list HTTP client")?;
    Ok(Router::new()
        .merge(list_route(role.coordinators.clone(), role, client.clone())?)
        .merge(list_route(role.mailboxes.clone(), role, client)?))
}

pub(crate) fn list_route<E: ListEntry>(
    mut entries: Vec<E>,
    role: &ServiceRegistryRole,
    client: reqwest::Client,
) -> anyhow::Result<Router> {
    let parent_registry_url = match role.parent_registry_url.as_deref() {
        Some(parent) => Some(
            curated_list::normalize_url(parent)
                .with_context(|| format!("parent_registry_url {parent:?}"))?,
        ),
        None => None,
    };
    curated_list::check_entries(&mut entries)?;
    info!(list = E::LIST_KEY, count = entries.len(), "serving curated list");
    let state = Arc::new(ListState {
        entries,
        max_age_secs: role.list_max_age_secs,
        parent_registry_url,
        client,
    });
    Ok(Router::new().route(&format!("/{}", E::LIST_KEY), get(serve_list::<E>)).with_state(state))
}

async fn serve_list<E: ListEntry>(
    State(state): State<Arc<ListState<E>>>,
    Query(filter): Query<E::Filter>,
    Query(hop): Query<HopQuery>,
) -> Result<Json<CuratedList<E>>, (StatusCode, String)> {
    let entries: Vec<E> = state.entries.iter().filter(|e| e.matches(&filter)).cloned().collect();
    let own = CuratedList { max_age_secs: state.max_age_secs, entries };
    if !own.entries.is_empty() || !E::FORWARDS_TO_PARENT {
        return Ok(Json(own));
    }
    let Some(parent) = state.parent_registry_url.as_deref() else {
        return Ok(Json(own));
    };
    if hop.hops >= curated_list::MAX_LIST_HOPS {
        debug!(list = E::LIST_KEY, hops = hop.hops, "hop limit reached; not forwarding");
        return Ok(Json(own));
    }
    match curated_list::fetch::<E>(&state.client, parent, &filter, hop.hops + 1).await {
        Ok(list) => Ok(Json(list)),
        Err(e) => {
            warn!(list = E::LIST_KEY, parent, error = %e, "parent registry did not answer the list");
            Err((StatusCode::BAD_GATEWAY, "parent registry did not answer".to_string()))
        }
    }
}

#[cfg(test)]
mod tests;
