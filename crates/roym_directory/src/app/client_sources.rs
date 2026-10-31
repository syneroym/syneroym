//! Client half: directory sources and publication to source.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::proxy::{CallOptions, CallTarget, ProxyError},
};
use syneroym_roym_core::{
    clock,
    directory::{DEFAULT_SOURCE_TIMEOUT_MS, MAX_SOURCES, SourceError},
    envelope::Response,
    method::{
        self, NoParams, Reply, SiblingError, call_sibling,
        catalog::{ListingGetOutput, ListingKeyInput},
        directory::{DirectoryInfoOutput, PublishOutput},
    },
    paging, person, services,
};

use super::{SOURCES, ensure_coll, put_json};
use crate::app;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SourceRow {
    pub(crate) did: String,
    pub(crate) label: String,
    pub(crate) added_at_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_ok_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_error: Option<SourceError>,
    /// Trust on first use, pinned once and never changed by a
    /// later reply. Absent until this node learns it (from an explicit
    /// `issuer_did` parameter, or from `directory.info`'s own field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) issuer_did: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AddSourceInput {
    pub did: String,
    pub label: Option<String>,
    pub issuer_did: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AddSourceOutput {
    pub source: SourceRow,
    pub probe: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SourcesOutput {
    pub sources: Vec<SourceRow>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublishToSourceInput {
    pub source: String,
    pub listing_id: String,
}

/// One `invoke` reply from a remote directory: its result read as `O`,
/// or its error returned unchanged.
fn remote_reply<O: DeserializeOwned>(raw: &str) -> Reply<O> {
    match method::read_reply::<O>(raw) {
        Ok(v) => Ok(v),
        Err(SiblingError::Refused(e)) => Err(Response { result: None, error: Some(e) }),
        Err(SiblingError::BadReply(text)) => {
            Err(Response::internal_error(format!("unreadable answer from the directory: {text}")))
        }
        Err(SiblingError::Transport(text)) => Err(Response::internal_error(text)),
    }
}

/// One `directory.info` call at a chosen address, over the wire, with no
/// side effect. What `add_source`'s own probe does; also its own verb, for
/// a caller (`roymctl roym directory info`) that wants to read a
/// directory's public statement without adding it as a source.
async fn probe_info<H: AppHost>(host: &H, did: &str) -> Result<String, ProxyError> {
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

#[derive(Default)]
struct ProbeRead {
    last_ok_secs: Option<u64>,
    last_error: Option<SourceError>,
    note: Option<String>,
    name: Option<String>,
    claimed_issuer: Option<String>,
}

fn read_probe(raw: &str, now: u64) -> ProbeRead {
    let v: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => {
            return ProbeRead {
                last_error: Some(SourceError::Unreadable { reason: e.to_string() }),
                ..Default::default()
            };
        }
    };
    if let Err(e) = serde_json::from_value::<Response>(v.clone()) {
        return ProbeRead {
            last_error: Some(SourceError::Unreadable { reason: e.to_string() }),
            ..Default::default()
        };
    }
    let res = v.get("result");
    if res.is_some_and(Value::is_null) {
        ProbeRead {
            last_ok_secs: Some(now),
            note: Some("this address answered, but runs no directory".to_string()),
            ..Default::default()
        }
    } else {
        let name = res.and_then(|r| r.get("name")).and_then(Value::as_str).map(str::to_string);
        let claimed_issuer =
            res.and_then(|r| r.get("issuer_did")).and_then(Value::as_str).map(str::to_string);
        ProbeRead { last_ok_secs: Some(now), last_error: None, note: None, name, claimed_issuer }
    }
}

pub(crate) async fn probe_info_verb<H: AppHost>(
    host: &H,
    input: app::DidInput,
) -> Reply<Option<DirectoryInfoOutput>> {
    if input.did.is_empty() {
        return Err(Response::invalid_params("did is required"));
    }
    match probe_info(host, &input.did).await {
        Ok(raw) => remote_reply::<Option<DirectoryInfoOutput>>(&raw),
        Err(e) => Err(Response::internal_error(format!("{e:?}"))),
    }
}

pub(crate) async fn add_source<H: AppHost>(
    host: &H,
    input: AddSourceInput,
) -> Reply<AddSourceOutput> {
    if input.did.is_empty() {
        return Err(Response::invalid_params("did is required"));
    }
    let did = input.did;
    let explicit_issuer = match input.issuer_did {
        None => None,
        Some(s) if person::is_did_key(&s) => Some(s),
        Some(_) => return Err(Response::invalid_params("issuer_did must be a did:key")),
    };
    if let Err(e) = ensure_coll(host, SOURCES, &[]).await {
        return Err(Response::internal_error(e));
    }
    let existing: Vec<(String, Value)> = match paging::query_all_with_ids(host, SOURCES, None).await
    {
        Ok(v) => v,
        Err(e) => return Err(Response::internal_error(e)),
    };
    let already_present = existing.iter().any(|(id, _)| id == &did);
    if !already_present && existing.len() >= MAX_SOURCES {
        return Err(Response::invalid_params(format!(
            "at most {MAX_SOURCES} sources may be added"
        )));
    }
    let existing_row: Option<SourceRow> = existing
        .iter()
        .find(|(id, _)| id == &did)
        .and_then(|(_, v)| serde_json::from_value(v.clone()).ok());

    let now = clock::now_secs();

    // Probe once: `directory.info` over the wire. A transport failure is
    // stored as `last_error`; a successful probe that answers `null` is
    // reported to the caller rather than silently accepted, so a person
    // does not sit waiting for results from an address that will never
    // have any.
    let probe = match probe_info(host, &did).await {
        Ok(raw) => read_probe(&raw, now),
        Err(_) => ProbeRead { last_error: Some(SourceError::TimedOut), ..Default::default() },
    };

    let mut label = input.label.unwrap_or_default();
    if label.is_empty()
        && let Some(name) = probe.name
    {
        label = name;
    }
    let mut probe_note = probe.note;
    let info_issuer = probe.claimed_issuer;

    // An explicit `issuer_did` wins; failing that, re-adding an
    // already-known source keeps its existing pin (a reply never re-pins
    // silently); only then does a first-time probe's own claim pin it.
    let issuer_did = explicit_issuer
        .clone()
        .or_else(|| existing_row.and_then(|r| r.issuer_did))
        .or(info_issuer.clone());
    if let (Some(explicit), Some(claimed)) = (&explicit_issuer, &info_issuer)
        && explicit != claimed
    {
        probe_note =
            Some(format!("this directory says it signs as {claimed}; you chose {explicit}"));
    }

    let row = SourceRow {
        did: did.clone(),
        label,
        added_at_secs: now,
        last_ok_secs: probe.last_ok_secs,
        last_error: probe.last_error,
        issuer_did,
    };
    if let Err(e) = put_json(host, SOURCES, &did, &row).await {
        return Err(Response::internal_error(e));
    }
    Ok(AddSourceOutput { source: row, probe: probe_note })
}

pub(crate) async fn remove_source<H: AppHost>(
    host: &H,
    input: app::DidInput,
) -> Reply<app::RemovedOutput> {
    let did = input.did;
    if let Err(e) = ensure_coll(host, SOURCES, &[]).await {
        return Err(Response::internal_error(e));
    }
    let existed = AppDataLayer::get(host, SOURCES.to_string(), did.clone())
        .await
        .map(|o| o.is_some())
        .unwrap_or(false);
    if existed && let Err(e) = AppDataLayer::delete(host, SOURCES.to_string(), did).await {
        return Err(Response::internal_error(e.to_string()));
    }
    Ok(app::RemovedOutput { removed: existed })
}

pub(crate) async fn sources<H: AppHost>(host: &H, _input: NoParams) -> Reply<SourcesOutput> {
    if let Err(e) = ensure_coll(host, SOURCES, &[]).await {
        return Err(Response::internal_error(e));
    }
    let sources: Vec<SourceRow> = match paging::query_all(host, SOURCES, None).await {
        Ok(r) => r,
        Err(e) => return Err(Response::internal_error(e)),
    };
    Ok(SourcesOutput { sources })
}

/// Push one of this installation's own listings to a directory. The
/// `source` is not required to be a registered search source: publishing
/// to a directory and searching one are two separate lists on purpose --
/// a listing owner may publish to a guild directory they never search, and
/// search directories they never publish to. The target is dialled
/// directly; the far end's `directory.publish` verifies the envelope and
/// enforces its own rate limit, so an unknown target costs this node
/// nothing beyond one outbound call. The caller is already this node's
/// own owner (the verb is local-only), and the envelope comes from the
/// local catalog, so there is no third party to protect here.
pub(crate) async fn publish_to_source<H: AppHost>(
    host: &H,
    input: PublishToSourceInput,
) -> Reply<PublishOutput> {
    let source = input.source;
    let listing_id = input.listing_id;

    let got = call_sibling::<_, ListingKeyInput, Option<ListingGetOutput>>(
        host,
        services::CATALOG,
        "listing.get",
        &ListingKeyInput { listing_id: listing_id.clone() },
    )
    .await;
    let listing = match got {
        Ok(Some(l)) => l,
        Ok(None) | Err(SiblingError::Refused(_)) => {
            return Err(Response::invalid_params("no such listing on this installation"));
        }
        Err(SiblingError::Transport(e) | SiblingError::BadReply(e)) => {
            return Err(Response::internal_error(e));
        }
    };
    let envelope = listing.envelope;

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
            return Err(Response::internal_error(format!(
                "directory.publish at '{source}': {e:?}"
            )));
        }
    };
    remote_reply::<PublishOutput>(&raw)
}
