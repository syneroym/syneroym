//! Request record operations: set, get, list, history.

use std::cmp::Reverse;

use serde::Deserialize;
use serde_json::{Map, Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost, AppSigning,
    types::{data_layer::QueryOptions, signing::RecordDraft},
};
use syneroym_roym_core::{
    area::Area,
    card::{self, CARD_CONTENT_TYPE},
    clock,
    envelope::{Request, Response},
    record::{Envelope, RECORD_REQUEST},
    transaction::{self, REQUEST_VERSION, RecordVerdict, RequestPayload, TimeWindow},
};

use super::{
    ListParams, REQUEST_HISTORY, REQUESTS, RecordPointerRow, conversation_call, count_mine,
    ensure_collections, file_own_card, get_row, put_bytes, put_row, resolve_principal_and_owner,
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

    let (sequence, supersedes, next_count) = if let Some(ref id) = params.request_id {
        let prior: RecordPointerRow = match get_row(host, REQUESTS, id).await {
            Ok(Some(p)) => p,
            Ok(None) => return Response::invalid_params("no such request"),
            Err(e) => return Response::internal_error(e),
        };
        if !prior.mine {
            return Response::invalid_params("this request is not yours to revise");
        }
        if prior.conversation != params.conversation {
            return Response::invalid_params("request belongs to another conversation");
        }
        (prior.sequence, Some(prior.record_id), prior.version_count + 1)
    } else {
        let count = match count_mine(host, REQUESTS, &params.conversation, &owner).await {
            Ok(c) => c,
            Err(e) => return Response::internal_error(e),
        };
        (count + 1, None, 1)
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

    let payload_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let draft = RecordDraft {
        version: REQUEST_VERSION,
        record_type: RECORD_REQUEST.to_string(),
        subject: request_id.clone(),
        payload: payload_str,
        expires_at_secs: None,
        supersedes,
    };

    let envelope_json = match AppSigning::sign_record(host, draft, principal).await {
        Ok(json) => json,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let envelope = match Envelope::from_json(&envelope_json) {
        Ok(env) => env,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    if envelope.issuer != owner {
        return Response::internal_error(
            "the host signed under an issuer this service did not ask for",
        );
    }

    let record_id = match envelope.record_id() {
        Ok(id) => id,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    // The record is stored before the card is sent. A send that fails leaves a
    // signed record this node holds and the peer does not, which a later `set`
    // or a retry repairs; the reverse -- a card the peer holds and this node
    // cannot show -- has no repair.
    if let Err(e) = put_bytes(host, REQUEST_HISTORY, &record_id, envelope_json.as_bytes()).await {
        return Response::internal_error(e);
    }

    let pointer = RecordPointerRow {
        envelope: envelope_json.clone(),
        record_id: record_id.clone(),
        id: request_id.clone(),
        conversation: params.conversation.clone(),
        sequence,
        issuer: owner.clone(),
        mine: true,
        updated_at_secs: now,
        version_count: next_count,
        issued_at_secs: now,
        request_record_id: None,
        consumer_did: None,
        declined_at_secs: None,
        decline_note: None,
    };
    if let Err(e) = put_row(host, REQUESTS, &request_id, &pointer).await {
        return Response::internal_error(e);
    }

    let body = match card::card_body(RECORD_REQUEST, REQUEST_VERSION, &envelope_json) {
        Ok(b) => b,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let send_resp = conversation_call(
        host,
        "conversation.send",
        json!({
            "conversation": params.conversation,
            "body": body,
            "content_type": CARD_CONTENT_TYPE,
        }),
    )
    .await;

    let (message_id, state, sender_timestamp_ms, mut send_error) = match send_resp {
        Ok(r) if r.error.is_none() => {
            let res = r.result.unwrap_or(Value::Null);
            let mid = res.get("message_id").and_then(Value::as_str).unwrap_or("").to_string();
            let st = res.get("state").and_then(Value::as_str).unwrap_or("").to_string();
            let ts =
                res.get("sender_timestamp_ms").and_then(Value::as_i64).unwrap_or(now as i64 * 1000);
            (mid, st, ts, None)
        }
        Ok(r) => {
            let err_msg = r.error.map(|e| e.message).unwrap_or_else(|| "send error".to_string());
            (String::new(), "not-sent".to_string(), now as i64 * 1000, Some(err_msg))
        }
        Err(e) => (String::new(), "not-sent".to_string(), now as i64 * 1000, Some(e)),
    };

    if !message_id.is_empty()
        && let Err(e) = file_own_card(
            host,
            &message_id,
            &params.conversation,
            RECORD_REQUEST,
            REQUEST_VERSION,
            &envelope_json,
            sender_timestamp_ms,
            now,
            Some(next_count),
        )
        .await
    {
        send_error = send_error.or(Some(format!("file own card failed: {e}")));
    }

    let mut out = json!({
        "request_id": request_id,
        "record_id": record_id,
        "version_count": next_count,
        "message_id": message_id,
        "state": state,
    });
    if let Some(err) = send_error {
        out["send_error"] = json!(err);
    }
    Response::ok(out)
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

    let mut filter_obj = Map::new();
    if let Some(ref c) = params.conversation {
        filter_obj.insert("conversation".to_string(), json!(c));
    }
    if let Some(m) = params.mine {
        filter_obj.insert("mine".to_string(), json!(m));
    }
    let filter =
        if filter_obj.is_empty() { None } else { Some(Value::Object(filter_obj).to_string()) };

    let mut rows = Vec::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            REQUESTS.to_string(),
            QueryOptions { filter: filter.clone(), limit: Some(500), cursor: cursor.clone() },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            if let Ok(p) = serde_json::from_slice::<RecordPointerRow>(&r.payload) {
                rows.push(p);
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
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
    let history_filter = json!({ "payload.request_id": params.request_id }).to_string();
    let mut envelopes: Vec<(u64, String)> = Vec::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            REQUEST_HISTORY.to_string(),
            QueryOptions {
                filter: Some(history_filter.clone()),
                limit: Some(500),
                cursor: cursor.clone(),
            },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            let env_str = String::from_utf8_lossy(&r.payload).into_owned();
            if let Ok(env) = Envelope::from_json(&env_str) {
                let matches = env
                    .payload
                    .get("request_id")
                    .and_then(Value::as_str)
                    .map(|id| id == params.request_id)
                    .unwrap_or(false);
                if matches {
                    envelopes.push((env.issued_at_secs, env_str));
                }
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    envelopes.sort_by_key(|(t, _)| *t);
    let out: Vec<Value> = envelopes.into_iter().map(|(_, e)| Value::String(e)).collect();
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
