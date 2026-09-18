//! Request record operations: set, get, list, history.

use std::cmp::Reverse;

use serde::Deserialize;
use serde_json::{Value, json};
use syneroym_app_host::{
    AppHost, AppSigning,
    types::signing::{Principal, RecordDraft},
};
use syneroym_roym_core::{
    area::Area,
    clock,
    envelope::{Request, Response},
    record::{Envelope, RECORD_REQUEST},
    transaction::{self, REQUEST_VERSION, RecordVerdict, RequestPayload, TimeWindow},
};

use super::{
    ListParams, REQUEST_HISTORY, REQUESTS, RecordPointerRow, collect_record_history, collect_typed,
    conversation_mine_filter, count_mine, ensure_collections, get_row, put_bytes, put_row,
    resolve_principal_and_owner, send_card_and_file,
};

#[derive(Debug, Deserialize)]
struct RequestSetParams {
    conversation: String,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    listing_id: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
    description: String,
    #[serde(default)]
    area: Option<Area>,
    #[serde(default)]
    window: Option<TimeWindow>,
    data_use_notice: String,
}

pub(crate) async fn request_set<H: AppHost>(host: &H, req: &Request) -> Response {
    let now = clock::now_secs();
    let (principal, owner) = match resolve_principal_and_owner(host, now).await {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };

    let params: RequestSetParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let (sequence, supersedes, next_count) =
        match resolve_request_sequence(host, &params, &owner).await {
            Ok(v) => v,
            Err(resp) => return resp,
        };

    let request_id = match transaction::derive_request_id(&params.conversation, &owner, sequence) {
        Ok(id) => id,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let payload = RequestPayload {
        request_id: request_id.clone(),
        conversation: params.conversation.clone(),
        sequence,
        listing_id: params.listing_id,
        categories: params.categories,
        description: params.description,
        area: params.area,
        window: params.window,
        data_use_notice: params.data_use_notice,
    };
    if let Err(e) = payload.validate() {
        return Response::invalid_params(e.to_string());
    }

    let signed = match sign_and_store_request(host, principal, &owner, &payload, supersedes).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    let pointer = build_request_pointer(&payload, &signed, &owner, next_count, now);
    if let Err(e) = put_row(host, REQUESTS, &request_id, &pointer).await {
        return Response::internal_error(e);
    }

    let (message_id, state, send_error) = send_card_and_file(
        host,
        &params.conversation,
        RECORD_REQUEST,
        REQUEST_VERSION,
        &signed.envelope_json,
        now,
        Some(next_count),
    )
    .await;

    let mut out = json!({
        "request_id": request_id,
        "record_id": signed.record_id,
        "version_count": next_count,
        "message_id": message_id,
        "state": state,
    });
    if let Some(err) = send_error {
        out["send_error"] = json!(err);
    }
    Response::ok(out)
}

/// Works out this request's sequence number and, when it revises an earlier
/// request, which record it supersedes.
async fn resolve_request_sequence<H: AppHost>(
    host: &H,
    params: &RequestSetParams,
    owner: &str,
) -> Result<(u32, Option<String>, u64), Response> {
    if let Some(ref id) = params.request_id {
        let prior: RecordPointerRow = match get_row(host, REQUESTS, id).await {
            Ok(Some(p)) => p,
            Ok(None) => return Err(Response::invalid_params("no such request")),
            Err(e) => return Err(Response::internal_error(e)),
        };
        if !prior.mine {
            return Err(Response::invalid_params("this request is not yours to revise"));
        }
        if prior.conversation != params.conversation {
            return Err(Response::invalid_params("request belongs to another conversation"));
        }
        Ok((prior.sequence, Some(prior.record_id), prior.version_count + 1))
    } else {
        let count = match count_mine(host, REQUESTS, &params.conversation, owner).await {
            Ok(c) => c,
            Err(e) => return Err(Response::internal_error(e)),
        };
        Ok((count + 1, None, 1))
    }
}

/// A freshly signed request envelope, plus the identifiers callers need to
/// reference it: kept as a struct rather than a `(String, String)` tuple so
/// the two values can't be swapped at a call site.
struct SignedRequest {
    record_id: String,
    envelope_json: String,
}

/// Serializes and signs `payload`, checks the host signed it under the
/// issuer this service asked for, and stores the envelope by its record id.
async fn sign_and_store_request<H: AppHost>(
    host: &H,
    principal: Principal,
    owner: &str,
    payload: &RequestPayload,
    supersedes: Option<String>,
) -> Result<SignedRequest, Response> {
    let payload_str = match serde_json::to_string(payload) {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let draft = RecordDraft {
        version: REQUEST_VERSION,
        record_type: RECORD_REQUEST.to_string(),
        subject: payload.request_id.clone(),
        payload: payload_str,
        expires_at_secs: None,
        supersedes,
    };

    let envelope_json = match AppSigning::sign_record(host, draft, principal).await {
        Ok(json) => json,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let envelope = match Envelope::from_json(&envelope_json) {
        Ok(env) => env,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    if envelope.issuer != owner {
        return Err(Response::internal_error(
            "the host signed under an issuer this service did not ask for",
        ));
    }

    let record_id = match envelope.record_id() {
        Ok(id) => id,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    // The record is stored before the card is sent. A send that fails leaves a
    // signed record this node holds and the peer does not, which a later `set`
    // or a retry repairs; the reverse -- a card the peer holds and this node
    // cannot show -- has no repair.
    if let Err(e) = put_bytes(host, REQUEST_HISTORY, &record_id, envelope_json.as_bytes()).await {
        return Err(Response::internal_error(e));
    }

    Ok(SignedRequest { record_id, envelope_json })
}

/// Builds the pointer row this node keeps to find the request again,
/// deriving the identity fields from `payload` so they can't drift from the
/// record that was actually signed.
fn build_request_pointer(
    payload: &RequestPayload,
    signed: &SignedRequest,
    owner: &str,
    next_count: u64,
    now: u64,
) -> RecordPointerRow {
    RecordPointerRow {
        envelope: signed.envelope_json.clone(),
        record_id: signed.record_id.clone(),
        id: payload.request_id.clone(),
        conversation: payload.conversation.clone(),
        sequence: payload.sequence,
        issuer: owner.to_string(),
        mine: true,
        updated_at_secs: now,
        version_count: next_count,
        issued_at_secs: now,
        request_record_id: None,
        consumer_did: None,
        declined_at_secs: None,
        decline_note: None,
    }
}

#[derive(Debug, Deserialize)]
struct RequestGetParams {
    request_id: String,
}

pub(crate) async fn request_get<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: RequestGetParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }
    let pointer: Option<RecordPointerRow> = match get_row(host, REQUESTS, &params.request_id).await
    {
        Ok(p) => p,
        Err(e) => return Response::internal_error(e),
    };
    match pointer {
        Some(p) => Response::ok(json!({
            "envelope": p.envelope,
            "record_id": p.record_id,
            "request_id": p.id,
            "conversation": p.conversation,
            "mine": p.mine,
            "updated_at_secs": p.updated_at_secs,
            "version_count": p.version_count,
        })),
        None => Response::ok(Value::Null),
    }
}

pub(crate) async fn request_list<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: ListParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let filter = conversation_mine_filter(params.conversation.as_deref(), params.mine);
    let mut rows: Vec<RecordPointerRow> = match collect_typed(host, REQUESTS, filter).await {
        Ok(rows) => rows,
        Err(e) => return e,
    };
    rows.sort_by_key(|p| Reverse(p.updated_at_secs));
    let page: Vec<Value> = rows
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .map(|p| {
            json!({
                "envelope": p.envelope,
                "record_id": p.record_id,
                "request_id": p.id,
                "conversation": p.conversation,
                "mine": p.mine,
                "updated_at_secs": p.updated_at_secs,
                "version_count": p.version_count,
            })
        })
        .collect();
    Response::ok(json!({ "requests": page }))
}

#[derive(Debug, Deserialize)]
struct RequestHistoryParams {
    request_id: String,
}

pub(crate) async fn request_history<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: RequestHistoryParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }
    let out = match collect_record_history(host, REQUEST_HISTORY, "request_id", &params.request_id)
        .await
    {
        Ok(out) => out,
        Err(e) => return e,
    };
    Response::ok(json!({ "history": out }))
}

pub(crate) async fn store_received_request<H: AppHost>(
    host: &H,
    envelope: &str,
    v: &RecordVerdict<RequestPayload>,
    payload: &RequestPayload,
    now: u64,
    owner: &str,
) -> Result<u64, String> {
    let record_id = v.record_id.as_deref().unwrap_or_default();
    let prior: Option<RecordPointerRow> = get_row(host, REQUESTS, &payload.request_id).await?;
    let incoming_issued = v.issued_at_secs.unwrap_or(now);
    if let Some(existing) = &prior
        && existing.record_id != record_id
        && incoming_issued <= existing.issued_at_secs
        && v.supersedes.as_deref() != Some(existing.record_id.as_str())
    {
        return Err("a newer or equal version of this request is already held here".to_string());
    }
    put_bytes(host, REQUEST_HISTORY, record_id, envelope.as_bytes()).await?;
    let version_count = if let Some(existing) = &prior {
        if existing.record_id == record_id {
            existing.version_count
        } else {
            existing.version_count + 1
        }
    } else {
        1
    };
    let mine = !owner.is_empty() && v.issuer.as_deref() == Some(owner);
    let pointer = RecordPointerRow {
        envelope: envelope.to_string(),
        record_id: record_id.to_string(),
        id: payload.request_id.clone(),
        conversation: payload.conversation.clone(),
        sequence: payload.sequence,
        issuer: v.issuer.clone().unwrap_or_default(),
        mine,
        updated_at_secs: now,
        version_count,
        issued_at_secs: incoming_issued,
        request_record_id: None,
        consumer_did: None,
        declined_at_secs: None,
        decline_note: None,
    };
    put_row(host, REQUESTS, &payload.request_id, &pointer).await?;
    Ok(version_count)
}
