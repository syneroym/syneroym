//! Quote record operations: set, get, list, history, decline, store.

use std::cmp::Reverse;

use serde::Deserialize;
use serde_json::{Value, json};
use syneroym_app_host::{
    AppHost, AppSigning,
    types::signing::{Principal, RecordDraft},
};
use syneroym_roym_core::{
    clock,
    envelope::{Request, Response},
    record::{Envelope, RECORD_QUOTE},
    signing::{self, CertificateError},
    transaction::{
        self, AgreedTerms, MAX_QUOTE_LIFETIME_SECS, MIN_QUOTE_LIFETIME_SECS, QUOTE_VERSION,
        QuotePayload, RecordVerdict, RequestPayload, TimeWindow,
    },
};

use super::{
    AGREEMENTS, AgreementRow, ListParams, QUOTE_HISTORY, QUOTES, REQUEST_HISTORY, RecordPointerRow,
    catalog_call, collect_record_history, collect_typed, conversation_mine_filter, count_mine,
    ensure_collections, get_bytes, get_row, put_bytes, put_row, resolve_principal_and_owner,
    send_card_and_file,
};

#[derive(Debug, Deserialize)]
struct QuoteSetParams {
    request_record_id: String,
    #[serde(default)]
    quote_id: Option<String>,
    #[serde(default)]
    listing_id: Option<String>,
    #[serde(default)]
    slot_id: Option<String>,
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

    let (req_payload, consumer_did) =
        match load_verified_request(host, &params.request_record_id, now, &owner).await {
            Ok(v) => v,
            Err(resp) => return resp,
        };
    let conversation = req_payload.conversation;

    let (sequence, supersedes, next_count) =
        match resolve_quote_sequence(host, &params, &conversation, &owner).await {
            Ok(v) => v,
            Err(resp) => return resp,
        };

    let expires_at_secs = now + params.expires_in_secs;
    let quote_id = match transaction::derive_quote_id(&conversation, &owner, sequence) {
        Ok(id) => id,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let mut terms = match parse_quote_terms(params.terms, expires_at_secs) {
        Ok(t) => t,
        Err(resp) => return resp,
    };

    if let Some(ref slot_id) = params.slot_id
        && let Err(resp) =
            validate_and_apply_slot(host, slot_id, params.listing_id.as_deref(), &mut terms).await
    {
        return resp;
    }

    let payload = QuotePayload {
        quote_id: quote_id.clone(),
        conversation: conversation.clone(),
        sequence,
        request_record_id: params.request_record_id.clone(),
        listing_id: params.listing_id,
        slot_id: params.slot_id,
        consumer_did: consumer_did.clone(),
        terms,
    };
    if let Err(e) = payload.validate() {
        return Response::invalid_params(e.to_string());
    }

    let signed =
        match sign_and_store_quote(host, principal, &owner, &payload, expires_at_secs, supersedes)
            .await
        {
            Ok(s) => s,
            Err(resp) => return resp,
        };

    let pointer = build_quote_pointer(&payload, &signed, &owner, next_count, now);
    if let Err(e) = put_row(host, QUOTES, &quote_id, &pointer).await {
        return Response::internal_error(e);
    }

    let (message_id, state, send_error) = send_card_and_file(
        host,
        &conversation,
        RECORD_QUOTE,
        QUOTE_VERSION,
        &signed.envelope_json,
        now,
        Some(next_count),
    )
    .await;

    let mut out = json!({
        "quote_id": quote_id,
        "record_id": signed.record_id,
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

/// Fetches and verifies the request this quote answers, and checks the
/// caller is not the same party that made the request.
async fn load_verified_request<H: AppHost>(
    host: &H,
    request_record_id: &str,
    now: u64,
    owner: &str,
) -> Result<(RequestPayload, String), Response> {
    let req_envelope_bytes = match get_bytes(host, REQUEST_HISTORY, request_record_id).await {
        Ok(Some(b)) => b,
        Ok(None) => return Err(Response::invalid_params("no such request")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let req_envelope_str = match String::from_utf8(req_envelope_bytes) {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let req_verdict = transaction::verify_request(&req_envelope_str, now);
    if !req_verdict.verified {
        return Err(Response::invalid_params(format!(
            "the request this quote answers does not verify: {}",
            req_verdict.reason.as_deref().unwrap_or("unknown")
        )));
    }
    let req_payload = match req_verdict.payload {
        Some(p) => p,
        None => return Err(Response::internal_error("missing request payload")),
    };
    let consumer_did = match req_verdict.issuer {
        Some(i) => i,
        None => return Err(Response::internal_error("missing request issuer")),
    };
    if consumer_did == owner {
        return Err(Response::invalid_params(
            "a request cannot be quoted by the person who made it",
        ));
    }
    Ok((req_payload, consumer_did))
}

/// Works out this quote's sequence number and, when it revises an earlier
/// quote, which record it supersedes.
async fn resolve_quote_sequence<H: AppHost>(
    host: &H,
    params: &QuoteSetParams,
    conversation: &str,
    owner: &str,
) -> Result<(u32, Option<String>, u64), Response> {
    if let Some(ref id) = params.quote_id {
        let prior: RecordPointerRow = match get_row(host, QUOTES, id).await {
            Ok(Some(p)) => p,
            Ok(None) => return Err(Response::invalid_params("no such quote")),
            Err(e) => return Err(Response::internal_error(e)),
        };
        if !prior.mine {
            return Err(Response::invalid_params("this quote is not yours to revise"));
        }
        if prior.conversation != conversation {
            return Err(Response::invalid_params("quote belongs to another conversation"));
        }
        if prior.request_record_id.as_deref() != Some(&params.request_record_id) {
            return Err(Response::invalid_params(
                "a new version of a quote answers the same request",
            ));
        }
        Ok((prior.sequence, Some(prior.record_id), prior.version_count + 1))
    } else {
        let count = match count_mine(host, QUOTES, conversation, owner).await {
            Ok(c) => c,
            Err(e) => return Err(Response::internal_error(e)),
        };
        Ok((count + 1, None, 1))
    }
}

/// Applies the caller-supplied expiry to the raw `terms` JSON and parses it.
fn parse_quote_terms(terms: Value, expires_at_secs: u64) -> Result<AgreedTerms, Response> {
    let mut terms_map = match terms {
        Value::Object(m) => m,
        _ => return Err(Response::invalid_params("terms must be an object")),
    };
    terms_map.insert("quote_expires_at_secs".to_string(), json!(expires_at_secs));
    serde_json::from_value(Value::Object(terms_map))
        .map_err(|e| Response::invalid_params(format!("invalid terms: {e}")))
}

async fn validate_and_apply_slot<H: AppHost>(
    host: &H,
    slot_id: &str,
    listing_id: Option<&str>,
    terms: &mut AgreedTerms,
) -> Result<(), Response> {
    let listing_id = match listing_id {
        Some(lid) => lid,
        None => return Err(Response::invalid_params("slot requires listing_id")),
    };
    let slot_resp =
        match catalog_call(host, "availability.get", json!({ "slot_id": slot_id })).await {
            Ok(r) => r,
            Err(e) => return Err(Response::internal_error(e)),
        };
    let slot = match slot_resp.result {
        Some(Value::Object(map)) => map,
        _ => return Err(Response::invalid_params("no-such-slot")),
    };
    if slot.get("listing_id").and_then(Value::as_str) != Some(listing_id) {
        return Err(Response::invalid_params("slot-not-in-listing"));
    }
    let start_secs = slot.get("start_secs").and_then(Value::as_u64).unwrap_or(0);
    let end_secs = slot.get("end_secs").and_then(Value::as_u64).unwrap_or(0);
    if let Some(ref sched) = terms.schedule
        && (sched.earliest_secs != start_secs || sched.latest_secs != end_secs)
    {
        return Err(Response::invalid_params("schedule-differs-from-slot"));
    }
    terms.schedule = Some(TimeWindow { earliest_secs: start_secs, latest_secs: end_secs });
    Ok(())
}

/// A freshly signed quote envelope, plus the identifiers callers need to
/// reference it: kept as a struct rather than a `(String, String)` tuple so
/// the two values can't be swapped at a call site.
struct SignedQuote {
    record_id: String,
    envelope_json: String,
}

/// Serializes and signs `payload`, checks the host signed it under the
/// issuer this service asked for, and stores the envelope by its record id.
async fn sign_and_store_quote<H: AppHost>(
    host: &H,
    principal: Principal,
    owner: &str,
    payload: &QuotePayload,
    expires_at_secs: u64,
    supersedes: Option<String>,
) -> Result<SignedQuote, Response> {
    let payload_str = match serde_json::to_string(payload) {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let draft = RecordDraft {
        version: QUOTE_VERSION,
        record_type: RECORD_QUOTE.to_string(),
        subject: payload.quote_id.clone(),
        payload: payload_str,
        expires_at_secs: Some(expires_at_secs),
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

    if let Err(e) = put_bytes(host, QUOTE_HISTORY, &record_id, envelope_json.as_bytes()).await {
        return Err(Response::internal_error(e));
    }

    Ok(SignedQuote { record_id, envelope_json })
}

/// Builds the pointer row this node keeps to find the quote again, deriving
/// the identity fields from `payload` so they can't drift from the record
/// that was actually signed.
fn build_quote_pointer(
    payload: &QuotePayload,
    signed: &SignedQuote,
    owner: &str,
    next_count: u64,
    now: u64,
) -> RecordPointerRow {
    RecordPointerRow {
        envelope: signed.envelope_json.clone(),
        record_id: signed.record_id.clone(),
        id: payload.quote_id.clone(),
        conversation: payload.conversation.clone(),
        sequence: payload.sequence,
        issuer: owner.to_string(),
        mine: true,
        updated_at_secs: now,
        version_count: next_count,
        issued_at_secs: now,
        request_record_id: Some(payload.request_record_id.clone()),
        consumer_did: Some(payload.consumer_did.clone()),
        declined_at_secs: None,
        decline_note: None,
    }
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

    let filter = conversation_mine_filter(params.conversation.as_deref(), params.mine);
    let mut rows: Vec<RecordPointerRow> = match collect_typed(host, QUOTES, filter).await {
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
    let out = match collect_record_history(host, QUOTE_HISTORY, "quote_id", &params.quote_id).await
    {
        Ok(out) => out,
        Err(e) => return e,
    };
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
