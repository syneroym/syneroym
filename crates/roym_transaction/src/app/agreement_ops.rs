//! Agreement record operations: accept, get, list, verify, countersign.

use std::cmp::Reverse;

use serde::Deserialize;
use serde_json::{Value, json};
use syneroym_app_host::{
    AppHost, AppSigning,
    types::signing::{Principal, RecordDraft},
};
use syneroym_roym_core::{
    booking::BookingState,
    card::{self, CARD_CONTENT_TYPE},
    clock,
    envelope::{Request, Response},
    record::{Envelope, RECORD_AGREEMENT_RECEIPT},
    signing,
    transaction::{
        self, AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, QuotePayload, ReceiptHalf,
        RecordVerdict, Role, pair_state,
    },
};

use super::{
    AGREEMENTS, AgreementRow, QUOTE_HISTORY, booking_ops, collect_typed, conversation_call,
    default_list_limit, ensure_collections, file_own_card, get_bytes, get_row, put_row,
    resolve_principal_and_owner, send_card_and_file,
};

#[derive(Debug, Deserialize)]
struct AgreementAcceptParams {
    quote_record_id: String,
}

pub(crate) async fn agreement_accept<H: AppHost>(host: &H, req: &Request) -> Response {
    let now = clock::now_secs();
    let (principal, owner) = match resolve_principal_and_owner(host, now).await {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };

    let params: AgreementAcceptParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let (quote_payload, provider_did, role) =
        match load_verified_quote_and_role(host, &params.quote_record_id, now, &owner).await {
            Ok(v) => v,
            Err(resp) => return resp,
        };

    let mut row = match load_or_init_agreement_row(
        host,
        &params.quote_record_id,
        &quote_payload,
        &provider_did,
        now,
    )
    .await
    {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    if let Some(resp) = already_accepted_response(&row, role, &params.quote_record_id) {
        return resp;
    }

    let booking_view_val = if role == Role::Provider {
        match prepare_provider_booking(host, &row, &quote_payload, &params.quote_record_id, now)
            .await
        {
            Ok(v) => v,
            Err(resp) => return resp,
        }
    } else {
        Value::Null
    };

    let half =
        match sign_agreement_receipt(host, principal, &params.quote_record_id, &row, &owner, role)
            .await
        {
            Ok(h) => h,
            Err(resp) => return resp,
        };

    let envelope_json = half.envelope.clone();
    let record_id = half.record_id.clone();
    row.set_half(role, half);
    row.updated_at_secs = now;
    if let Err(e) = put_row(host, AGREEMENTS, &params.quote_record_id, &row).await {
        return Response::internal_error(e);
    }

    let (message_id, state, send_error) = send_card_and_file(
        host,
        &quote_payload.conversation,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &envelope_json,
        now,
        None,
    )
    .await;

    let pair = pair_state(row.consumer.as_ref(), row.provider.as_ref());
    let mut out = json!({
        "quote_record_id": params.quote_record_id,
        "role": role,
        "record_id": record_id,
        "pair": pair,
        "booking": booking_view_val,
        "message_id": message_id,
        "state": state,
    });
    if let Some(err) = send_error {
        out["send_error"] = json!(err);
    }
    Response::ok(out)
}

/// Fetches and verifies the quote a caller wants to accept, and works out
/// which side of it (consumer or provider) this installation's owner is.
async fn load_verified_quote_and_role<H: AppHost>(
    host: &H,
    quote_record_id: &str,
    now: u64,
    owner: &str,
) -> Result<(QuotePayload, String, Role), Response> {
    let quote_envelope_bytes = match get_bytes(host, QUOTE_HISTORY, quote_record_id).await {
        Ok(Some(b)) => b,
        Ok(None) => return Err(Response::invalid_params("no such quote")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let quote_envelope_str = match String::from_utf8(quote_envelope_bytes) {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let verdict = transaction::verify_quote(&quote_envelope_str, now);
    if !verdict.verified {
        return Err(Response::invalid_params(format!(
            "the quote does not verify: {}",
            verdict.reason.as_deref().unwrap_or("unknown")
        )));
    }
    if verdict.expired {
        return Err(Response::invalid_params("quote-expired"));
    }

    let quote_payload = match verdict.payload {
        Some(p) => p,
        None => return Err(Response::internal_error("missing quote payload")),
    };
    let provider_did = match verdict.issuer {
        Some(i) => i,
        None => return Err(Response::internal_error("missing quote issuer")),
    };

    let role = if owner == quote_payload.consumer_did {
        Role::Consumer
    } else if owner == provider_did {
        Role::Provider
    } else {
        return Err(Response::invalid_params("this installation is neither party to that quote"));
    };

    Ok((quote_payload, provider_did, role))
}

/// Loads the agreement row this quote already has, or seeds a fresh one from
/// the quote's own payload when this is the first half either party files.
async fn load_or_init_agreement_row<H: AppHost>(
    host: &H,
    quote_record_id: &str,
    quote_payload: &QuotePayload,
    provider_did: &str,
    now: u64,
) -> Result<AgreementRow, Response> {
    match get_row(host, AGREEMENTS, quote_record_id).await {
        Ok(Some(r)) => Ok(r),
        Ok(None) => Ok(AgreementRow {
            quote_record_id: quote_record_id.to_string(),
            conversation: quote_payload.conversation.clone(),
            consumer_did: quote_payload.consumer_did.clone(),
            provider_did: provider_did.to_string(),
            terms: quote_payload.terms.clone(),
            consumer: None,
            provider: None,
            updated_at_secs: now,
        }),
        Err(e) => Err(Response::internal_error(e)),
    }
}

/// Some(response) when this role already has a half on file for this
/// agreement, so the caller can return the idempotent "already-accepted"
/// reply instead of signing a second one.
fn already_accepted_response(
    row: &AgreementRow,
    role: Role,
    quote_record_id: &str,
) -> Option<Response> {
    let existing = row.half(role)?;
    let pair = pair_state(row.consumer.as_ref(), row.provider.as_ref());
    Some(Response::ok(json!({
        "quote_record_id": quote_record_id,
        "role": role,
        "record_id": existing.record_id,
        "pair": pair,
        "message_id": "",
        "state": "already-accepted",
    })))
}

/// Builds, validates and signs this role's half of the agreement receipt,
/// and checks the host signed it under the issuer this service asked for.
async fn sign_agreement_receipt<H: AppHost>(
    host: &H,
    principal: Principal,
    quote_record_id: &str,
    row: &AgreementRow,
    owner: &str,
    role: Role,
) -> Result<ReceiptHalf, Response> {
    let payload = AgreementReceiptPayload {
        quote_record_id: quote_record_id.to_string(),
        consumer_did: row.consumer_did.clone(),
        provider_did: row.provider_did.clone(),
        role,
        terms: row.terms.clone(),
    };
    if let Err(e) = payload.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }

    let payload_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let draft = RecordDraft {
        version: AGREEMENT_RECEIPT_VERSION,
        record_type: RECORD_AGREEMENT_RECEIPT.to_string(),
        subject: quote_record_id.to_string(),
        payload: payload_str,
        expires_at_secs: None,
        supersedes: None,
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

    Ok(ReceiptHalf {
        envelope: envelope_json,
        record_id,
        issuer: owner.to_string(),
        issued_at_secs: envelope.issued_at_secs,
    })
}

#[derive(Debug, Deserialize)]
struct AgreementGetParams {
    quote_record_id: String,
}

pub(crate) async fn agreement_get<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: AgreementGetParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }
    let row: Option<AgreementRow> = match get_row(host, AGREEMENTS, &params.quote_record_id).await {
        Ok(r) => r,
        Err(e) => return Response::internal_error(e),
    };
    match row {
        Some(r) => {
            let pair = pair_state(r.consumer.as_ref(), r.provider.as_ref());
            Response::ok(json!({
                "quote_record_id": r.quote_record_id,
                "conversation": r.conversation,
                "consumer_did": r.consumer_did,
                "provider_did": r.provider_did,
                "consumer": r.consumer,
                "provider": r.provider,
                "pair": pair,
                "terms": r.terms,
            }))
        }
        None => Response::ok(Value::Null),
    }
}

#[derive(Debug, Deserialize)]
struct AgreementListParams {
    #[serde(default)]
    conversation: Option<String>,
    #[serde(default = "default_list_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}

pub(crate) async fn agreement_list<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: AgreementListParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let filter = params.conversation.map(|c| json!({ "conversation": c }).to_string());
    let mut rows: Vec<AgreementRow> = match collect_typed(host, AGREEMENTS, filter).await {
        Ok(rows) => rows,
        Err(e) => return e,
    };
    rows.sort_by_key(|a| Reverse(a.updated_at_secs));
    let page: Vec<Value> = rows
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .map(|r| {
            let pair = pair_state(r.consumer.as_ref(), r.provider.as_ref());
            json!({
                "quote_record_id": r.quote_record_id,
                "conversation": r.conversation,
                "consumer_did": r.consumer_did,
                "provider_did": r.provider_did,
                "consumer": r.consumer,
                "provider": r.provider,
                "pair": pair,
                "terms": r.terms,
            })
        })
        .collect();
    Response::ok(json!({ "agreements": page }))
}

async fn prepare_provider_booking<H: AppHost>(
    host: &H,
    row: &AgreementRow,
    quote_payload: &QuotePayload,
    quote_record_id: &str,
    now: u64,
) -> Result<Value, Response> {
    if quote_payload.slot_id.is_some() && row.consumer.is_none() {
        return Err(Response::invalid_params(
            "provider cannot accept a slot quote before the consumer accepts",
        ));
    }
    if row.consumer.is_some() {
        let b = match booking_ops::decide_booking(host, row, quote_payload, now).await {
            Ok(b) => b,
            Err(e) => return Err(Response::internal_error(e)),
        };
        if b.state == BookingState::Conflict {
            return Err(Response::invalid_params("slot-unavailable"));
        }
        let view = booking_ops::booking_view(
            quote_record_id,
            Some(&b.snapshot),
            Some(&b.progress_record_id),
            Some("self"),
            row,
            Role::Provider,
        );
        return Ok(view);
    }
    Ok(Value::Null)
}

/// Attempts to countersign an agreement on the provider's node when the
/// consumer has accepted. Refused, and left to the person, when the slot the
/// quote names is already full.
pub(crate) async fn maybe_countersign<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    qv: &RecordVerdict<QuotePayload>,
    now: u64,
    owner: &str,
) -> Result<bool, String> {
    if row.consumer.is_none() {
        return Ok(false);
    }
    let q_payload = match qv.payload.as_ref() {
        Some(p) => p,
        None => return Ok(false),
    };
    if row.provider.is_some() {
        let _ = booking_ops::decide_booking(host, row, q_payload, now).await;
        return Ok(false);
    }
    if owner.is_empty() || owner != row.provider_did {
        return Ok(false);
    }
    if now >= row.terms.quote_expires_at_secs {
        return Ok(false);
    }
    let booking = booking_ops::decide_booking(host, row, q_payload, now).await?;
    if booking.state == BookingState::Conflict {
        return Ok(false);
    }
    let (principal, _master) = match signing::person_principal(host, now).await {
        Ok(p) => p,
        Err(_) => return Ok(false),
    };

    let payload = AgreementReceiptPayload {
        quote_record_id: row.quote_record_id.clone(),
        consumer_did: row.consumer_did.clone(),
        provider_did: row.provider_did.clone(),
        role: Role::Provider,
        terms: q_payload.terms.clone(),
    };
    if payload.validate().is_err() {
        return Ok(false);
    }
    let payload_str = serde_json::to_string(&payload).map_err(|e| e.to_string())?;

    let draft = RecordDraft {
        version: AGREEMENT_RECEIPT_VERSION,
        record_type: RECORD_AGREEMENT_RECEIPT.to_string(),
        subject: row.quote_record_id.clone(),
        payload: payload_str,
        expires_at_secs: None,
        supersedes: None,
    };

    let envelope_json = match AppSigning::sign_record(host, draft, principal).await {
        Ok(json) => json,
        Err(e) => return Err(e.to_string()),
    };

    let envelope = Envelope::from_json(&envelope_json).map_err(|e| e.to_string())?;
    let record_id = envelope.record_id().map_err(|e| e.to_string())?;

    let half = ReceiptHalf {
        envelope: envelope_json.clone(),
        record_id: record_id.clone(),
        issuer: owner.to_string(),
        issued_at_secs: envelope.issued_at_secs,
    };
    row.provider = Some(half);
    row.updated_at_secs = now;
    put_row(host, AGREEMENTS, &row.quote_record_id, row).await?;

    let body = card::card_body(RECORD_AGREEMENT_RECEIPT, AGREEMENT_RECEIPT_VERSION, &envelope_json)
        .map_err(|e| e.to_string())?;

    let send_resp = conversation_call(
        host,
        "conversation.send",
        json!({
            "conversation": row.conversation,
            "body": body,
            "content_type": CARD_CONTENT_TYPE,
        }),
    )
    .await;

    let (message_id, sender_timestamp_ms) = match send_resp {
        Ok(r) if r.error.is_none() => {
            let res = r.result.unwrap_or(Value::Null);
            let mid = res.get("message_id").and_then(Value::as_str).unwrap_or("").to_string();
            let ts =
                res.get("sender_timestamp_ms").and_then(Value::as_i64).unwrap_or(now as i64 * 1000);
            (mid, ts)
        }
        _ => (String::new(), now as i64 * 1000),
    };

    if !message_id.is_empty() {
        file_own_card(
            host,
            &message_id,
            &row.conversation,
            RECORD_AGREEMENT_RECEIPT,
            AGREEMENT_RECEIPT_VERSION,
            &envelope_json,
            sender_timestamp_ms,
            now,
            None,
        )
        .await?;
    }

    Ok(true)
}
