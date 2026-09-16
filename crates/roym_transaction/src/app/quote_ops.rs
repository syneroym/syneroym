//! Quote record operations: set, get, list, history, decline, store.

use std::cmp::Reverse;

use serde::Deserialize;
use serde_json::{Map, Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost, AppSigning,
    types::{data_layer::QueryOptions, signing::RecordDraft},
};
use syneroym_roym_core::{
    card::{self, CARD_CONTENT_TYPE},
    clock,
    envelope::{Request, Response},
    record::{Envelope, RECORD_QUOTE},
    signing::{self, CertificateError},
    transaction::{
        self, AgreedTerms, MAX_QUOTE_LIFETIME_SECS, MIN_QUOTE_LIFETIME_SECS, QUOTE_VERSION,
        QuotePayload, RecordVerdict,
    },
};

use super::{
    AGREEMENTS, AgreementRow, ListParams, QUOTE_HISTORY, QUOTES, REQUEST_HISTORY, RecordPointerRow,
    conversation_call, count_mine, ensure_collections, file_own_card, get_bytes, get_row,
    put_bytes, put_row, resolve_principal_and_owner,
};

#[derive(Debug, Deserialize)]
struct QuoteSetParams {
    request_record_id: String,
    #[serde(default)]
    quote_id: Option<String>,
    #[serde(default)]
    listing_id: Option<String>,
    expires_in_secs: u64,
    terms: Value,
}

pub(crate) async fn quote_set<H: AppHost>(host: &H, req: &Request) -> Response {
    let now = clock::now_secs();
    let (principal, owner) = match resolve_principal_and_owner(host, now).await {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };

    let params: QuoteSetParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    if !(MIN_QUOTE_LIFETIME_SECS..=MAX_QUOTE_LIFETIME_SECS).contains(&params.expires_in_secs) {
        return Response::invalid_params(format!(
            "expires_in_secs must be between {MIN_QUOTE_LIFETIME_SECS} and \
             {MAX_QUOTE_LIFETIME_SECS}"
        ));
    }

    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let req_envelope_bytes = match get_bytes(host, REQUEST_HISTORY, &params.request_record_id).await
    {
        Ok(Some(b)) => b,
        Ok(None) => return Response::invalid_params("no such request"),
        Err(e) => return Response::internal_error(e),
    };

    let req_envelope_str = match String::from_utf8(req_envelope_bytes) {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let req_verdict = transaction::verify_request(&req_envelope_str, now);
    if !req_verdict.verified {
        return Response::invalid_params(format!(
            "the request this quote answers does not verify: {}",
            req_verdict.reason.as_deref().unwrap_or("unknown")
        ));
    }
    let req_payload = match req_verdict.payload {
        Some(p) => p,
        None => return Response::internal_error("missing request payload"),
    };
    let consumer_did = match req_verdict.issuer {
        Some(i) => i,
        None => return Response::internal_error("missing request issuer"),
    };
    if consumer_did == owner {
        return Response::invalid_params("a request cannot be quoted by the person who made it");
    }
    let conversation = req_payload.conversation;

    let (sequence, supersedes, next_count) = if let Some(ref id) = params.quote_id {
        let prior: RecordPointerRow = match get_row(host, QUOTES, id).await {
            Ok(Some(p)) => p,
            Ok(None) => return Response::invalid_params("no such quote"),
            Err(e) => return Response::internal_error(e),
        };
        if !prior.mine {
            return Response::invalid_params("this quote is not yours to revise");
        }
        if prior.conversation != conversation {
            return Response::invalid_params("quote belongs to another conversation");
        }
        if prior.request_record_id.as_deref() != Some(&params.request_record_id) {
            return Response::invalid_params("a new version of a quote answers the same request");
        }
        (prior.sequence, Some(prior.record_id), prior.version_count + 1)
    } else {
        let count = match count_mine(host, QUOTES, &conversation, &owner).await {
            Ok(c) => c,
            Err(e) => return Response::internal_error(e),
        };
        (count + 1, None, 1)
    };

    let expires_at_secs = now + params.expires_in_secs;
    let quote_id = match transaction::derive_quote_id(&conversation, &owner, sequence) {
        Ok(id) => id,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let mut terms_map = match params.terms {
        Value::Object(m) => m,
        _ => return Response::invalid_params("terms must be an object"),
    };
    terms_map.insert("quote_expires_at_secs".to_string(), json!(expires_at_secs));
    let terms: AgreedTerms = match serde_json::from_value(Value::Object(terms_map)) {
        Ok(t) => t,
        Err(e) => return Response::invalid_params(format!("invalid terms: {e}")),
    };

    let payload = QuotePayload {
        quote_id: quote_id.clone(),
        conversation: conversation.clone(),
        sequence,
        request_record_id: params.request_record_id.clone(),
        listing_id: params.listing_id,
        consumer_did: consumer_did.clone(),
        terms,
    };
    if let Err(e) = payload.validate() {
        return Response::invalid_params(e.to_string());
    }

    let payload_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let draft = RecordDraft {
        version: QUOTE_VERSION,
        record_type: RECORD_QUOTE.to_string(),
        subject: quote_id.clone(),
        payload: payload_str,
        expires_at_secs: Some(expires_at_secs),
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

    if let Err(e) = put_bytes(host, QUOTE_HISTORY, &record_id, envelope_json.as_bytes()).await {
        return Response::internal_error(e);
    }

    let pointer = RecordPointerRow {
        envelope: envelope_json.clone(),
        record_id: record_id.clone(),
        id: quote_id.clone(),
        conversation: conversation.clone(),
        sequence,
        issuer: owner.clone(),
        mine: true,
        updated_at_secs: now,
        version_count: next_count,
        issued_at_secs: now,
        request_record_id: Some(params.request_record_id),
        consumer_did: Some(consumer_did),
        declined_at_secs: None,
        decline_note: None,
    };
    if let Err(e) = put_row(host, QUOTES, &quote_id, &pointer).await {
        return Response::internal_error(e);
    }

    let body = match card::card_body(RECORD_QUOTE, QUOTE_VERSION, &envelope_json) {
        Ok(b) => b,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let send_resp = conversation_call(
        host,
        "conversation.send",
        json!({
            "conversation": conversation,
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
            &conversation,
            RECORD_QUOTE,
            QUOTE_VERSION,
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
        "quote_id": quote_id,
        "record_id": record_id,
        "version_count": next_count,
        "expires_at_secs": expires_at_secs,
        "message_id": message_id,
        "state": state,
    });
    if let Some(err) = send_error {
        out["send_error"] = json!(err);
    }
    Response::ok(out)
}

#[derive(Debug, Deserialize)]
struct QuoteGetParams {
    quote_id: String,
}

pub(crate) async fn quote_get<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: QuoteGetParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }
    let pointer: Option<RecordPointerRow> = match get_row(host, QUOTES, &params.quote_id).await {
        Ok(p) => p,
        Err(e) => return Response::internal_error(e),
    };
    match pointer {
        Some(p) => Response::ok(json!({
            "envelope": p.envelope,
            "record_id": p.record_id,
            "quote_id": p.id,
            "conversation": p.conversation,
            "mine": p.mine,
            "updated_at_secs": p.updated_at_secs,
            "version_count": p.version_count,
            "request_record_id": p.request_record_id,
            "consumer_did": p.consumer_did,
            "declined_at_secs": p.declined_at_secs,
            "decline_note": p.decline_note,
        })),
        None => Response::ok(Value::Null),
    }
}

pub(crate) async fn quote_list<H: AppHost>(host: &H, req: &Request) -> Response {
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
            QUOTES.to_string(),
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
                "quote_id": p.id,
                "conversation": p.conversation,
                "mine": p.mine,
                "updated_at_secs": p.updated_at_secs,
                "version_count": p.version_count,
                "request_record_id": p.request_record_id,
                "consumer_did": p.consumer_did,
            })
        })
        .collect();
    Response::ok(json!({ "quotes": page }))
}

#[derive(Debug, Deserialize)]
struct QuoteHistoryParams {
    quote_id: String,
}

pub(crate) async fn quote_history<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: QuoteHistoryParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }
    let history_filter = json!({ "payload.quote_id": params.quote_id }).to_string();
    let mut envelopes: Vec<(u64, String)> = Vec::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            QUOTE_HISTORY.to_string(),
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
                    .get("quote_id")
                    .and_then(Value::as_str)
                    .map(|id| id == params.quote_id)
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

#[derive(Debug, Deserialize)]
struct QuoteDeclineParams {
    quote_record_id: String,
    #[serde(default)]
    note: Option<String>,
}

pub(crate) async fn quote_decline<H: AppHost>(host: &H, req: &Request) -> Response {
    let now = clock::now_secs();
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(CertificateError::NoOwner) => {
            return Response::invalid_params("this installation has no recorded owner");
        }
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let params: QuoteDeclineParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let quote_envelope_bytes = match get_bytes(host, QUOTE_HISTORY, &params.quote_record_id).await {
        Ok(Some(b)) => b,
        Ok(None) => return Response::invalid_params("no such quote"),
        Err(e) => return Response::internal_error(e),
    };

    let quote_envelope_str = match String::from_utf8(quote_envelope_bytes) {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let verdict = transaction::verify_quote(&quote_envelope_str, now);
    if !verdict.verified {
        return Response::invalid_params(format!(
            "the quote does not verify: {}",
            verdict.reason.as_deref().unwrap_or("unknown")
        ));
    }
    let quote_payload = match verdict.payload {
        Some(p) => p,
        None => return Response::internal_error("missing quote payload"),
    };

    if owner != quote_payload.consumer_did {
        return Response::invalid_params("only the consumer can decline this quote");
    }

    if let Ok(Some(agr)) =
        get_row::<AgreementRow, _>(host, AGREEMENTS, &params.quote_record_id).await
        && (agr.consumer.is_some() || agr.provider.is_some())
    {
        return Response::invalid_params("cannot decline a quote that has already been accepted");
    }

    let mut pointer: RecordPointerRow = match get_row(host, QUOTES, &quote_payload.quote_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return Response::invalid_params("no such quote"),
        Err(e) => return Response::internal_error(e),
    };

    pointer.declined_at_secs = Some(now);
    pointer.decline_note = params.note.clone();
    pointer.updated_at_secs = now;
    if let Err(e) = put_row(host, QUOTES, &quote_payload.quote_id, &pointer).await {
        return Response::internal_error(e);
    }

    Response::ok(json!({
        "quote_record_id": params.quote_record_id,
        "declined": true,
        "note": params.note,
    }))
}

pub(crate) async fn store_received_quote<H: AppHost>(
    host: &H,
    envelope: &str,
    v: &RecordVerdict<QuotePayload>,
    payload: &QuotePayload,
    now: u64,
    owner: &str,
) -> Result<u64, String> {
    let record_id = v.record_id.as_deref().unwrap_or_default();
    let prior: Option<RecordPointerRow> = get_row(host, QUOTES, &payload.quote_id).await?;
    let incoming_issued = v.issued_at_secs.unwrap_or(now);
    if let Some(existing) = &prior
        && existing.record_id != record_id
        && incoming_issued <= existing.issued_at_secs
        && v.supersedes.as_deref() != Some(existing.record_id.as_str())
    {
        return Err("a newer or equal version of this quote is already held here".to_string());
    }
    put_bytes(host, QUOTE_HISTORY, record_id, envelope.as_bytes()).await?;
    let (version_count, declined_at_secs, decline_note) = if let Some(existing) = &prior {
        if existing.record_id == record_id {
            // Idempotent replay: preserve decline and version count
            (existing.version_count, existing.declined_at_secs, existing.decline_note.clone())
        } else {
            // New version: fresh offer, not declined
            (existing.version_count + 1, None, None)
        }
    } else {
        (1, None, None)
    };
    let mine = !owner.is_empty() && v.issuer.as_deref() == Some(owner);
    let pointer = RecordPointerRow {
        envelope: envelope.to_string(),
        record_id: record_id.to_string(),
        id: payload.quote_id.clone(),
        conversation: payload.conversation.clone(),
        sequence: payload.sequence,
        issuer: v.issuer.clone().unwrap_or_default(),
        mine,
        updated_at_secs: now,
        version_count,
        issued_at_secs: incoming_issued,
        request_record_id: Some(payload.request_record_id.clone()),
        consumer_did: Some(payload.consumer_did.clone()),
        declined_at_secs,
        decline_note,
    };
    put_row(host, QUOTES, &payload.quote_id, &pointer).await?;
    Ok(version_count)
}
