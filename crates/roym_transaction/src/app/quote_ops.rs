//! Quote record operations: set, get, list, history, decline, store.

#[cfg(test)]
mod tests;

use std::cmp::Reverse;

use serde::{Deserialize, Serialize};
use syneroym_app_host::{
    AppHost, AppSigning,
    types::signing::{Principal, RecordDraft},
};
use syneroym_roym_core::{
    clock,
    envelope::Response,
    method::{
        self, EnvelopeHistoryOutput, Reply, SiblingError, VerifyInput, call_sibling,
        catalog::{SlotKeyInput, SlotRow},
    },
    paging,
    record::{Envelope, RECORD_QUOTE},
    services,
    signing::{self, CertificateError},
    transaction::{
        self, AgreedTerms, MAX_QUOTE_LIFETIME_SECS, MIN_QUOTE_LIFETIME_SECS, PaymentTiming,
        QUOTE_VERSION, QuoteLocation, QuotePayload, RecordVerdict, RequestPayload, TimeWindow,
    },
};

use super::{
    AGREEMENTS, AgreementRow, ListInput, QUOTE_HISTORY, QUOTES, REQUEST_HISTORY, RecordPointerRow,
    SendState, conversation_mine_filter, count_mine, ensure_collections, get_bytes, get_row,
    put_bytes, put_row, resolve_principal_and_owner, send_card_and_file,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteTermsInput {
    pub scope: String,
    pub currency: String,
    pub amount_minor: i64,
    pub tax_minor: i64,
    pub fees_minor: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub payment_methods: Vec<String>,
    pub payee: String,
    pub payment_timing: PaymentTiming,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<TimeWindow>,
    pub location: QuoteLocation,
    pub cancellation_terms: String,
    pub refund_terms: String,
    pub dispute_path: String,
}

impl QuoteTermsInput {
    pub fn into_agreed_terms(self, quote_expires_at_secs: u64) -> AgreedTerms {
        AgreedTerms {
            scope: self.scope,
            currency: self.currency,
            amount_minor: self.amount_minor,
            tax_minor: self.tax_minor,
            fees_minor: self.fees_minor,
            payment_methods: self.payment_methods,
            payee: self.payee,
            payment_timing: self.payment_timing,
            schedule: self.schedule,
            location: self.location,
            cancellation_terms: self.cancellation_terms,
            refund_terms: self.refund_terms,
            dispute_path: self.dispute_path,
            quote_expires_at_secs,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteSetInput {
    pub request_record_id: String,
    #[serde(default)]
    pub quote_id: Option<String>,
    #[serde(default)]
    pub listing_id: Option<String>,
    #[serde(default)]
    pub slot_id: Option<String>,
    pub expires_in_secs: u64,
    pub terms: QuoteTermsInput,
}

#[derive(Debug, Serialize)]
pub struct QuoteSetOutput {
    pub quote_id: String,
    pub record_id: String,
    pub version_count: u64,
    pub expires_at_secs: u64,
    pub message_id: String,
    pub state: SendState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteKeyInput {
    pub quote_id: String,
}

#[derive(Debug, Serialize)]
pub struct QuoteView {
    pub envelope: String,
    pub record_id: String,
    pub quote_id: String,
    pub conversation: String,
    pub mine: bool,
    pub updated_at_secs: u64,
    pub version_count: u64,
    pub request_record_id: Option<String>,
    pub counterparty_did: Option<String>,
    pub declined_at_secs: Option<u64>,
    pub decline_note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct QuoteListItem {
    pub envelope: String,
    pub record_id: String,
    pub quote_id: String,
    pub conversation: String,
    pub mine: bool,
    pub updated_at_secs: u64,
    pub version_count: u64,
    pub request_record_id: Option<String>,
    pub counterparty_did: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct QuoteListOutput {
    pub quotes: Vec<QuoteListItem>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteDeclineInput {
    pub quote_record_id: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct QuoteDeclineOutput {
    pub quote_record_id: String,
    pub declined: bool,
    pub note: Option<String>,
}

pub(crate) async fn set<H: AppHost>(host: &H, input: QuoteSetInput) -> Reply<QuoteSetOutput> {
    let now = clock::now_secs();
    let (principal, owner) = resolve_principal_and_owner(host, now).await?;

    if !(MIN_QUOTE_LIFETIME_SECS..=MAX_QUOTE_LIFETIME_SECS).contains(&input.expires_in_secs) {
        return Err(Response::invalid_params(format!(
            "expires_in_secs must be between {MIN_QUOTE_LIFETIME_SECS} and \
             {MAX_QUOTE_LIFETIME_SECS}"
        )));
    }

    ensure_collections(host).await.map_err(Response::internal_error)?;

    let (req_payload, counterparty_did) =
        load_verified_request(host, &input.request_record_id, now, &owner).await?;
    let conversation = req_payload.conversation;

    let (sequence, supersedes, next_count) =
        resolve_quote_sequence(host, &input, &conversation, &owner).await?;

    let expires_at_secs = now + input.expires_in_secs;
    let quote_id = match transaction::derive_quote_id(&conversation, &owner, sequence) {
        Ok(id) => id,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let mut terms = input.terms.into_agreed_terms(expires_at_secs);

    if let Some(ref slot_id) = input.slot_id {
        validate_and_apply_slot(host, slot_id, input.listing_id.as_deref(), &mut terms).await?;
    }

    let payload = QuotePayload {
        quote_id: quote_id.clone(),
        conversation: conversation.clone(),
        sequence,
        request_record_id: input.request_record_id.clone(),
        listing_id: input.listing_id,
        slot_id: input.slot_id,
        counterparty_did: counterparty_did.clone(),
        terms,
    };
    if let Err(e) = payload.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }

    let signed =
        sign_and_store_quote(host, principal, &owner, &payload, expires_at_secs, supersedes)
            .await?;

    let pointer = build_quote_pointer(&payload, &signed, &owner, next_count, now);
    if let Err(e) = put_row(host, QUOTES, &quote_id, &pointer).await {
        return Err(Response::internal_error(e));
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

    Ok(QuoteSetOutput {
        quote_id,
        record_id: signed.record_id,
        version_count: next_count,
        expires_at_secs,
        message_id,
        state,
        send_error,
    })
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
    let counterparty_did = match req_verdict.issuer {
        Some(i) => i,
        None => return Err(Response::internal_error("missing request issuer")),
    };
    if counterparty_did == owner {
        return Err(Response::invalid_params(
            "a request cannot be quoted by the person who made it",
        ));
    }
    Ok((req_payload, counterparty_did))
}

/// Works out this quote's sequence number and, when it revises an earlier
/// quote, which record it supersedes.
async fn resolve_quote_sequence<H: AppHost>(
    host: &H,
    params: &QuoteSetInput,
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
    let slot = match call_sibling::<_, SlotKeyInput, Option<SlotRow>>(
        host,
        services::CATALOG,
        "availability.get",
        &SlotKeyInput { slot_id: slot_id.to_string() },
    )
    .await
    {
        Ok(Some(s)) => s,
        Ok(None) | Err(SiblingError::Refused(_) | SiblingError::BadReply(_)) => {
            return Err(Response::invalid_params("no-such-slot"));
        }
        Err(SiblingError::Transport(e)) => return Err(Response::internal_error(e)),
    };
    if slot.listing_id != listing_id {
        return Err(Response::invalid_params("slot-not-in-listing"));
    }
    let start_secs = slot.start_secs;
    let end_secs = slot.end_secs;
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
        counterparty_did: Some(payload.counterparty_did.clone()),
        declined_at_secs: None,
        decline_note: None,
    }
}

pub(crate) async fn get<H: AppHost>(host: &H, input: QuoteKeyInput) -> Reply<Option<QuoteView>> {
    ensure_collections(host).await.map_err(Response::internal_error)?;
    let pointer: Option<RecordPointerRow> = match get_row(host, QUOTES, &input.quote_id).await {
        Ok(p) => p,
        Err(e) => return Err(Response::internal_error(e)),
    };
    Ok(pointer.map(|p| QuoteView {
        envelope: p.envelope,
        record_id: p.record_id,
        quote_id: p.id,
        conversation: p.conversation,
        mine: p.mine,
        updated_at_secs: p.updated_at_secs,
        version_count: p.version_count,
        request_record_id: p.request_record_id,
        counterparty_did: p.counterparty_did,
        declined_at_secs: p.declined_at_secs,
        decline_note: p.decline_note,
    }))
}

pub(crate) async fn list<H: AppHost>(host: &H, input: ListInput) -> Reply<QuoteListOutput> {
    ensure_collections(host).await.map_err(Response::internal_error)?;

    let filter = conversation_mine_filter(input.conversation.as_deref(), input.mine);
    let mut rows: Vec<RecordPointerRow> = match paging::query_all(host, QUOTES, filter).await {
        Ok(rows) => rows,
        Err(e) => return Err(Response::internal_error(e)),
    };
    rows.sort_by_key(|p| Reverse(p.updated_at_secs));
    let offset = usize::try_from(input.offset).unwrap_or(usize::MAX);
    let limit = usize::try_from(input.limit).unwrap_or(usize::MAX);
    let page: Vec<QuoteListItem> = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|p| QuoteListItem {
            envelope: p.envelope,
            record_id: p.record_id,
            quote_id: p.id,
            conversation: p.conversation,
            mine: p.mine,
            updated_at_secs: p.updated_at_secs,
            version_count: p.version_count,
            request_record_id: p.request_record_id,
            counterparty_did: p.counterparty_did,
        })
        .collect();
    Ok(QuoteListOutput { quotes: page })
}

pub(crate) async fn history<H: AppHost>(
    host: &H,
    input: QuoteKeyInput,
) -> Reply<EnvelopeHistoryOutput> {
    ensure_collections(host).await.map_err(Response::internal_error)?;
    let out = match paging::envelope_history(host, QUOTE_HISTORY, "quote_id", &input.quote_id).await
    {
        Ok(out) => out,
        Err(e) => return Err(Response::internal_error(e)),
    };
    Ok(EnvelopeHistoryOutput { history: out })
}

pub(crate) async fn decline<H: AppHost>(
    host: &H,
    input: QuoteDeclineInput,
) -> Reply<QuoteDeclineOutput> {
    let now = clock::now_secs();
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(CertificateError::NoOwner) => {
            return Err(Response::invalid_params("this installation has no recorded owner"));
        }
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    ensure_collections(host).await.map_err(Response::internal_error)?;

    let quote_envelope_bytes = match get_bytes(host, QUOTE_HISTORY, &input.quote_record_id).await {
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
    let quote_payload = match verdict.payload {
        Some(p) => p,
        None => return Err(Response::internal_error("missing quote payload")),
    };

    if owner != quote_payload.counterparty_did {
        return Err(Response::invalid_params("only the counterparty can decline this quote"));
    }

    if let Ok(Some(agr)) =
        get_row::<AgreementRow, _>(host, AGREEMENTS, &input.quote_record_id).await
        && (agr.counterparty.is_some() || agr.quote_author.is_some())
    {
        return Err(Response::invalid_params(
            "cannot decline a quote that has already been accepted",
        ));
    }

    let mut pointer: RecordPointerRow = match get_row(host, QUOTES, &quote_payload.quote_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return Err(Response::invalid_params("no such quote")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    pointer.declined_at_secs = Some(now);
    pointer.decline_note = input.note.clone();
    pointer.updated_at_secs = now;
    if let Err(e) = put_row(host, QUOTES, &quote_payload.quote_id, &pointer).await {
        return Err(Response::internal_error(e));
    }

    Ok(QuoteDeclineOutput {
        quote_record_id: input.quote_record_id,
        declined: true,
        note: input.note,
    })
}

pub(crate) async fn verify<H: AppHost>(
    _host: &H,
    input: VerifyInput,
) -> Reply<RecordVerdict<QuotePayload>> {
    method::verify_with(input, transaction::verify_quote)
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
        counterparty_did: Some(payload.counterparty_did.clone()),
        declined_at_secs,
        decline_note,
    };
    put_row(host, QUOTES, &payload.quote_id, &pointer).await?;
    Ok(version_count)
}
