//! Client half: directory sources and publication to source.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::proxy::{CallOptions, CallTarget, ProxyError},
};
use syneroym_roym_core::{
    clock,
    directory::{DEFAULT_SOURCE_TIMEOUT_MS, MAX_SOURCES, SourceError},
    envelope::{Request, Response},
    services,
};

use super::{SOURCES, collect_raw, ensure_coll, put_json};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SourceRow {
    pub(crate) did: String,
    pub(crate) label: String,
    pub(crate) added_at_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_ok_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_error: Option<SourceError>,
}

/// One `directory.info` call at a chosen address, over the wire, with no
/// side effect. What `add_source`'s own probe does; also its own verb, for
/// a caller (`roymctl roym directory info`) that wants to read a
/// directory's public statement without adding it as a source.
pub(crate) async fn probe_info<H: AppHost>(host: &H, did: &str) -> Result<String, ProxyError> {
    let probe_params = json!({ "method": "directory.info", "params": {} }).to_string();
    host.call(
        CallTarget::Service(did.to_string()),
        services::DIRECTORY.interface.to_string(),
        "invoke".to_string(),
        json!([probe_params]).to_string(),
        Some(CallOptions {
            protocol: None,
            idempotent: true,
            timeout_ms: Some(DEFAULT_SOURCE_TIMEOUT_MS),
            routing_key: None,
            idempotency_key: None,
        }),
    )
    .await
}

pub(crate) async fn probe_info_verb<H: AppHost>(host: &H, req: &Request) -> Response {
    let did = match req.params.get("did").and_then(Value::as_str) {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => return Response::invalid_params("did is required"),
    };
    match probe_info(host, &did).await {
        Ok(raw) => match serde_json::from_str::<Response>(&raw) {
            Ok(resp) => resp,
            Err(e) => Response::internal_error(e.to_string()),
        },
        Err(e) => Response::internal_error(format!("{e:?}")),
    }
}

pub(crate) async fn add_source<H: AppHost>(host: &H, req: &Request) -> Response {
    let did = match req.params.get("did").and_then(Value::as_str) {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => return Response::invalid_params("did is required"),
    };
    if let Err(e) = ensure_coll(host, SOURCES, &[]).await {
        return Response::internal_error(e);
    }
    let existing = match collect_raw(host, SOURCES).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let already_present = existing.iter().any(|(id, _)| id == &did);
    if !already_present && existing.len() >= MAX_SOURCES {
        return Response::invalid_params(format!("at most {MAX_SOURCES} sources may be added"));
    }

    // Probe once: `directory.info` over the wire. A transport failure is
    // stored as `last_error`; a successful probe that answers `null` is
    // reported to the caller rather than silently accepted, so a person
    // does not sit waiting for results from an address that will never
    // have any.
    let probe = probe_info(host, &did).await;

    let now = clock::now_secs();
    let requested_label =
        req.params.get("label").and_then(Value::as_str).unwrap_or_default().to_string();
    let mut last_ok_secs = None;
    let mut last_error = None;
    let mut probe_note: Option<String> = None;
    let mut label = requested_label.clone();
    match probe {
        Ok(raw) => match serde_json::from_str::<Response>(&raw) {
            Ok(resp) if resp.result.as_ref().is_some_and(Value::is_null) => {
                last_ok_secs = Some(now);
                probe_note = Some("this address answered, but runs no directory".to_string());
            }
            Ok(resp) => {
                last_ok_secs = Some(now);
                if label.is_empty() {
                    label = resp
                        .result
                        .as_ref()
                        .and_then(|r| r.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                }
            }
            Err(e) => last_error = Some(SourceError::Unreadable { reason: e.to_string() }),
        },
        Err(_) => last_error = Some(SourceError::TimedOut),
    };

    let row = SourceRow { did: did.clone(), label, added_at_secs: now, last_ok_secs, last_error };
    if let Err(e) = put_json(host, SOURCES, &did, &row).await {
        return Response::internal_error(e);
    }
    Response::ok(json!({ "source": row, "probe": probe_note }))
}

pub(crate) async fn remove_source<H: AppHost>(host: &H, req: &Request) -> Response {
    let did = match req.params.get("did").and_then(Value::as_str) {
        Some(d) => d.to_string(),
        None => return Response::invalid_params("did is required"),
    };
    if let Err(e) = ensure_coll(host, SOURCES, &[]).await {
        return Response::internal_error(e);
    }
    let existed = AppDataLayer::get(host, SOURCES.to_string(), did.clone())
        .await
        .map(|o| o.is_some())
        .unwrap_or(false);
    if existed && let Err(e) = AppDataLayer::delete(host, SOURCES.to_string(), did).await {
        return Response::internal_error(e.to_string());
    }
    Response::ok(json!({ "removed": existed }))
}

pub(crate) async fn sources<H: AppHost>(host: &H) -> Response {
    if let Err(e) = ensure_coll(host, SOURCES, &[]).await {
        return Response::internal_error(e);
    }
    match collect_raw(host, SOURCES).await {
        Ok(rows) => {
            Response::ok(json!({ "sources": rows.into_iter().map(|(_, v)| v).collect::<Vec<_>>() }))
        }
        Err(e) => Response::internal_error(e),
    }
}

/// Push one of this installation's own listings to a directory. The
/// `source` is not required to be a registered search source: publishing
/// to a directory and searching one are two separate lists on purpose --
/// a provider may publish to a guild directory they never search, and
/// search directories they never publish to. The target is dialled
/// directly; the far end's `directory.publish` verifies the envelope and
/// enforces its own rate limit, so an unknown target costs this node
/// nothing beyond one outbound call. The caller is already this node's
/// own owner (the verb is local-only), and the envelope comes from the
/// local catalog, so there is no third party to protect here.
pub(crate) async fn publish_to_source<H: AppHost>(host: &H, req: &Request) -> Response {
    let source = match req.params.get("source").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => return Response::invalid_params("source is required"),
    };
    let listing_id = match req.params.get("listing_id").and_then(Value::as_str) {
        Some(l) => l.to_string(),
        None => return Response::invalid_params("listing_id is required"),
    };

    let get_req =
        json!({ "method": "listing.get", "params": { "listing_id": listing_id } }).to_string();
    let raw = match host
        .call(
            CallTarget::Dependency(services::CATALOG.name.to_string()),
            services::CATALOG.interface.to_string(),
            "invoke".to_string(),
            json!([get_req]).to_string(),
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(e) => return Response::internal_error(format!("listing.get: {e:?}")),
    };
    let resp: Response = match serde_json::from_str(&raw) {
        Ok(r) => r,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    let Some(result) = resp.result.filter(|r| !r.is_null()) else {
        return Response::invalid_params("no such listing on this installation");
    };
    let envelope = match result.get("envelope").and_then(Value::as_str) {
        Some(e) => e.to_string(),
        None => return Response::internal_error("listing.get returned no envelope"),
    };

    let publish_req =
        json!({ "method": "directory.publish", "params": { "envelope": envelope } }).to_string();
    let raw = match host
        .call(
            CallTarget::Service(source.clone()),
            services::DIRECTORY.interface.to_string(),
            "invoke".to_string(),
            json!([publish_req]).to_string(),
            Some(CallOptions {
                protocol: None,
                idempotent: false,
                timeout_ms: Some(DEFAULT_SOURCE_TIMEOUT_MS),
                routing_key: None,
                idempotency_key: None,
            }),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return Response::internal_error(format!("directory.publish at '{source}': {e:?}"));
        }
    };
    match serde_json::from_str::<Response>(&raw) {
        Ok(r) => r,
        Err(e) => Response::internal_error(e.to_string()),
    }
}
