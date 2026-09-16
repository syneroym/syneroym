//! Agreement record operations: accept, get, list, verify, countersign.

use std::cmp::Reverse;

use serde::Deserialize;
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost, AppSigning,
    types::{data_layer::QueryOptions, signing::RecordDraft},
};
use syneroym_roym_core::{
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
    AGREEMENTS, AgreementRow, QUOTE_HISTORY, conversation_call, default_list_limit,
    ensure_collections, file_own_card, get_bytes, get_row, put_row, resolve_principal_and_owner,
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
    if verdict.expired {
        return Response::invalid_params("quote-expired");
    }

    let quote_payload = match verdict.payload {
        Some(p) => p,
        None => return Response::internal_error("missing quote payload"),
    };
    let provider_did = match verdict.issuer {
        Some(i) => i,
        None => return Response::internal_error("missing quote issuer"),
    };
    let consumer_did = quote_payload.consumer_did;

    let role = if owner == consumer_did {
        Role::Consumer
    } else if owner == provider_did {
        Role::Provider
    } else {
        return Response::invalid_params("this installation is neither party to that quote");
    };

    let mut row: AgreementRow = match get_row(host, AGREEMENTS, &params.quote_record_id).await {
        Ok(Some(r)) => r,
        Ok(None) => AgreementRow {
            quote_record_id: params.quote_record_id.clone(),
            conversation: quote_payload.conversation.clone(),
            consumer_did: consumer_did.clone(),
            provider_did: provider_did.clone(),
            terms: quote_payload.terms.clone(),
            consumer: None,
            provider: None,
            updated_at_secs: now,
        },
        Err(e) => return Response::internal_error(e),
    };

    if let Some(existing) = row.half(role) {
        let pair = pair_state(row.consumer.as_ref(), row.provider.as_ref());
        return Response::ok(json!({
            "quote_record_id": params.quote_record_id,
            "role": role,
            "record_id": existing.record_id,
            "pair": pair,
            "message_id": "",
            "state": "already-accepted",
        }));
    }

    let payload = AgreementReceiptPayload {
        quote_record_id: params.quote_record_id.clone(),
        consumer_did: consumer_did.clone(),
        provider_did: provider_did.clone(),
        role,
        terms: quote_payload.terms.clone(),
    };
    if let Err(e) = payload.validate() {
        return Response::invalid_params(e.to_string());
    }

    let payload_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let draft = RecordDraft {
        version: AGREEMENT_RECEIPT_VERSION,
        record_type: RECORD_AGREEMENT_RECEIPT.to_string(),
        subject: params.quote_record_id.clone(),
        payload: payload_str,
        expires_at_secs: None,
        supersedes: None,
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

    let half = ReceiptHalf {
        envelope: envelope_json.clone(),
        record_id: record_id.clone(),
        issuer: owner.clone(),
        issued_at_secs: envelope.issued_at_secs,
    };

    row.set_half(role, half);
    row.updated_at_secs = now;
    if let Err(e) = put_row(host, AGREEMENTS, &params.quote_record_id, &row).await {
        return Response::internal_error(e);
    }

    let body = match card::card_body(
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &envelope_json,
    ) {
        Ok(b) => b,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let send_resp = conversation_call(
        host,
        "conversation.send",
        json!({
            "conversation": quote_payload.conversation,
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
            &quote_payload.conversation,
            RECORD_AGREEMENT_RECEIPT,
            AGREEMENT_RECEIPT_VERSION,
            &envelope_json,
            sender_timestamp_ms,
            now,
            None,
        )
        .await
    {
        send_error = send_error.or(Some(format!("file own card failed: {e}")));
    }

    let pair = pair_state(row.consumer.as_ref(), row.provider.as_ref());
    let mut out = json!({
        "quote_record_id": params.quote_record_id,
        "role": role,
        "record_id": record_id,
        "pair": pair,
        "message_id": message_id,
        "state": state,
    });
    if let Some(err) = send_error {
        out["send_error"] = json!(err);
    }
    Response::ok(out)
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
    let mut rows = Vec::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            AGREEMENTS.to_string(),
            QueryOptions { filter: filter.clone(), limit: Some(500), cursor: cursor.clone() },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            if let Ok(a) = serde_json::from_slice::<AgreementRow>(&r.payload) {
                rows.push(a);
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordKind {
    Request,
    Quote,
    AgreementReceipt,
}

pub(crate) async fn verify_verb<H: AppHost>(host: &H, req: &Request, kind: RecordKind) -> Response {
    let _ = host;
    let now = clock::now_secs();
    let env_val = match req.params.get("envelope") {
        Some(v) => v,
        None => return Response::invalid_params("envelope is required"),
    };
    let env_str = match env_val {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    match kind {
        RecordKind::Request => Response::ok(json!(transaction::verify_request(&env_str, now))),
        RecordKind::Quote => Response::ok(json!(transaction::verify_quote(&env_str, now))),
        RecordKind::AgreementReceipt => {
            Response::ok(json!(transaction::verify_agreement_receipt(&env_str, now)))
        }
    }
}

pub(crate) async fn maybe_countersign<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    qv: &RecordVerdict<QuotePayload>,
    now: u64,
    owner: &str,
) -> Result<bool, String> {
    if row.provider.is_some() {
        return Ok(false);
    }
    if row.consumer.is_none() {
        return Ok(false);
    }
    if owner.is_empty() || owner != row.provider_did {
        return Ok(false);
    }
    let q_payload = match qv.payload.as_ref() {
        Some(p) => p,
        None => return Ok(false),
    };
    if now >= row.terms.quote_expires_at_secs {
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
