//! Agreement record operations: accept, get, list, verify, countersign.

use std::cmp::Reverse;

use serde::{Deserialize, Serialize};
use syneroym_app_host::{
    AppHost, AppSigning,
    types::signing::{Principal, RecordDraft},
};
use syneroym_roym_core::{
    booking::BookingState,
    card::{self, CARD_CONTENT_TYPE},
    clock,
    envelope::Response,
    method::{
        self, Reply, VerifyInput,
        conversation::{SendInput, SendOutput},
    },
    paging,
    record::{Envelope, RECORD_AGREEMENT_RECEIPT},
    services, signing,
    transaction::{
        self, AGREEMENT_RECEIPT_VERSION, AgreedTerms, AgreementReceiptPayload, DealRole, PairState,
        QuotePayload, ReceiptHalf, RecordVerdict, pair_state,
    },
};

use super::{
    AGREEMENTS, AgreementRow, BookingWriter, QUOTE_HISTORY, SendState,
    booking_ops::{self, BookingView},
    default_list_limit_u64, ensure_collections, file_own_card, get_bytes, get_row, put_row,
    resolve_principal_and_owner, send_card_and_file,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteRecordInput {
    pub quote_record_id: String,
}

#[derive(Debug, Serialize)]
pub struct AgreementAcceptOutput {
    pub quote_record_id: String,
    pub role: DealRole,
    pub record_id: String,
    pub pair: PairState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub booking: Option<Option<BookingView>>,
    pub message_id: String,
    pub state: SendState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AgreementView {
    pub quote_record_id: String,
    pub conversation: String,
    pub counterparty_did: String,
    pub quote_author_did: String,
    pub counterparty: Option<ReceiptHalf>,
    pub quote_author: Option<ReceiptHalf>,
    pub pair: PairState,
    pub terms: AgreedTerms,
}

impl From<AgreementRow> for AgreementView {
    fn from(r: AgreementRow) -> Self {
        let pair = pair_state(r.counterparty.as_ref(), r.quote_author.as_ref());
        Self {
            quote_record_id: r.quote_record_id,
            conversation: r.conversation,
            counterparty_did: r.counterparty_did,
            quote_author_did: r.quote_author_did,
            counterparty: r.counterparty,
            quote_author: r.quote_author,
            pair,
            terms: r.terms,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgreementListInput {
    #[serde(default)]
    pub conversation: Option<String>,
    #[serde(default = "default_list_limit_u64")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

#[derive(Debug, Serialize)]
pub struct AgreementListOutput {
    pub agreements: Vec<AgreementView>,
}

pub(crate) async fn accept<H: AppHost>(
    host: &H,
    input: QuoteRecordInput,
) -> Reply<AgreementAcceptOutput> {
    let now = clock::now_secs();
    let (principal, owner) = resolve_principal_and_owner(host, now).await?;

    ensure_collections(host).await.map_err(Response::internal_error)?;

    let (quote_payload, quote_author_did, role) =
        load_verified_quote_and_role(host, &input.quote_record_id, now, &owner).await?;

    let mut row = load_or_init_agreement_row(
        host,
        &input.quote_record_id,
        &quote_payload,
        &quote_author_did,
        now,
    )
    .await?;

    if let Some(out) = already_accepted_response(&row, role, &input.quote_record_id) {
        return Ok(out);
    }

    let booking_view = if role == DealRole::QuoteAuthor {
        prepare_author_booking(host, &row, &quote_payload, &input.quote_record_id, now).await?
    } else {
        None
    };

    let half =
        sign_agreement_receipt(host, principal, &input.quote_record_id, &row, &owner, role).await?;

    let envelope_json = half.envelope.clone();
    let record_id = half.record_id.clone();
    row.set_half(role, half);
    row.updated_at_secs = now;
    if let Err(e) = put_row(host, AGREEMENTS, &input.quote_record_id, &row).await {
        return Err(Response::internal_error(e));
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

    let pair = pair_state(row.counterparty.as_ref(), row.quote_author.as_ref());
    Ok(AgreementAcceptOutput {
        quote_record_id: input.quote_record_id,
        role,
        record_id,
        pair,
        booking: Some(booking_view),
        message_id,
        state,
        send_error,
    })
}

/// Fetches and verifies the quote a caller wants to accept, and works out
/// which side of it (counterparty or quote author) this installation's owner
/// is.
async fn load_verified_quote_and_role<H: AppHost>(
    host: &H,
    quote_record_id: &str,
    now: u64,
    owner: &str,
) -> Result<(QuotePayload, String, DealRole), Response> {
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
    let quote_author_did = match verdict.issuer {
        Some(i) => i,
        None => return Err(Response::internal_error("missing quote issuer")),
    };

    let role = if owner == quote_payload.counterparty_did {
        DealRole::Counterparty
    } else if owner == quote_author_did {
        DealRole::QuoteAuthor
    } else {
        return Err(Response::invalid_params("this installation is neither party to that quote"));
    };

    Ok((quote_payload, quote_author_did, role))
}

/// Loads the agreement row this quote already has, or seeds a fresh one from
/// the quote's own payload when this is the first half either party files.
async fn load_or_init_agreement_row<H: AppHost>(
    host: &H,
    quote_record_id: &str,
    quote_payload: &QuotePayload,
    quote_author_did: &str,
    now: u64,
) -> Result<AgreementRow, Response> {
    match get_row(host, AGREEMENTS, quote_record_id).await {
        Ok(Some(r)) => Ok(r),
        Ok(None) => Ok(AgreementRow {
            quote_record_id: quote_record_id.to_string(),
            conversation: quote_payload.conversation.clone(),
            counterparty_did: quote_payload.counterparty_did.clone(),
            quote_author_did: quote_author_did.to_string(),
            terms: quote_payload.terms.clone(),
            counterparty: None,
            quote_author: None,
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
    role: DealRole,
    quote_record_id: &str,
) -> Option<AgreementAcceptOutput> {
    let existing = row.half(role)?;
    let pair = pair_state(row.counterparty.as_ref(), row.quote_author.as_ref());
    Some(AgreementAcceptOutput {
        quote_record_id: quote_record_id.to_string(),
        role,
        record_id: existing.record_id.clone(),
        pair,
        booking: None,
        message_id: String::new(),
        state: SendState::AlreadyAccepted,
        send_error: None,
    })
}

/// Builds, validates and signs this role's half of the agreement receipt,
/// and checks the host signed it under the issuer this service asked for.
async fn sign_agreement_receipt<H: AppHost>(
    host: &H,
    principal: Principal,
    quote_record_id: &str,
    row: &AgreementRow,
    owner: &str,
    role: DealRole,
) -> Result<ReceiptHalf, Response> {
    let payload = AgreementReceiptPayload {
        quote_record_id: quote_record_id.to_string(),
        counterparty_did: row.counterparty_did.clone(),
        quote_author_did: row.quote_author_did.clone(),
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

pub(crate) async fn get<H: AppHost>(
    host: &H,
    input: QuoteRecordInput,
) -> Reply<Option<AgreementView>> {
    ensure_collections(host).await.map_err(Response::internal_error)?;
    let row: Option<AgreementRow> = match get_row(host, AGREEMENTS, &input.quote_record_id).await {
        Ok(r) => r,
        Err(e) => return Err(Response::internal_error(e)),
    };
    Ok(row.map(AgreementView::from))
}

pub(crate) async fn list<H: AppHost>(
    host: &H,
    input: AgreementListInput,
) -> Reply<AgreementListOutput> {
    ensure_collections(host).await.map_err(Response::internal_error)?;

    let filter =
        input.conversation.as_ref().map(|c| serde_json::json!({ "conversation": c }).to_string());
    let mut rows: Vec<AgreementRow> = match paging::query_all(host, AGREEMENTS, filter).await {
        Ok(rows) => rows,
        Err(e) => return Err(Response::internal_error(e)),
    };
    rows.sort_by_key(|a| Reverse(a.updated_at_secs));
    let offset = usize::try_from(input.offset).unwrap_or(usize::MAX);
    let limit = usize::try_from(input.limit).unwrap_or(usize::MAX);
    let page: Vec<AgreementView> =
        rows.into_iter().skip(offset).take(limit).map(AgreementView::from).collect();
    Ok(AgreementListOutput { agreements: page })
}

pub(crate) async fn verify<H: AppHost>(
    _host: &H,
    input: VerifyInput,
) -> Reply<RecordVerdict<AgreementReceiptPayload>> {
    method::verify_with(input, transaction::verify_agreement_receipt)
}

async fn prepare_author_booking<H: AppHost>(
    host: &H,
    row: &AgreementRow,
    quote_payload: &QuotePayload,
    quote_record_id: &str,
    now: u64,
) -> Result<Option<BookingView>, Response> {
    if quote_payload.slot_id.is_some() && row.counterparty.is_none() {
        return Err(Response::invalid_params(
            "the quote author cannot accept a slot quote before the counterparty accepts",
        ));
    }
    if row.counterparty.is_some() {
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
            Some(BookingWriter::Mine),
            row,
            DealRole::QuoteAuthor,
        );
        return Ok(Some(view));
    }
    Ok(None)
}

/// Attempts to countersign an agreement on the quote author's node when the
/// counterparty has accepted. Refused, and left to the person, when the slot
/// the quote names is already full.
pub(crate) async fn maybe_countersign<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    qv: &RecordVerdict<QuotePayload>,
    now: u64,
    owner: &str,
) -> Result<bool, String> {
    if row.counterparty.is_none() {
        return Ok(false);
    }
    let q_payload = match qv.payload.as_ref() {
        Some(p) => p,
        None => return Ok(false),
    };
    if row.quote_author.is_some() {
        let _ = booking_ops::decide_booking(host, row, q_payload, now).await;
        return Ok(false);
    }
    if owner.is_empty() || owner != row.quote_author_did {
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
        counterparty_did: row.counterparty_did.clone(),
        quote_author_did: row.quote_author_did.clone(),
        role: DealRole::QuoteAuthor,
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
    row.quote_author = Some(half);
    row.updated_at_secs = now;
    put_row(host, AGREEMENTS, &row.quote_record_id, &row).await?;

    let body = card::card_body(RECORD_AGREEMENT_RECEIPT, AGREEMENT_RECEIPT_VERSION, &envelope_json)
        .map_err(|e| e.to_string())?;

    let send_resp = method::call_sibling::<_, SendInput, SendOutput>(
        host,
        services::CONVERSATION,
        "conversation.send",
        &SendInput {
            conversation: row.conversation.clone(),
            body,
            content_type: Some(CARD_CONTENT_TYPE.to_string()),
        },
    )
    .await;

    let (message_id, sender_timestamp_ms) = match send_resp {
        Ok(out) => (out.message_id, out.sender_timestamp_ms),
        Err(_) => (String::new(), now as i64 * 1000),
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
