//! Quote record operations: set, get, list, history, decline, store.

pub mod input;
#[cfg(test)]
mod tests;

use std::cmp::Reverse;

pub use input::{CommitmentInput, DealTermsInput};
use serde::{Deserialize, Serialize};
use syneroym_app_host::{
    AppHost, AppSigning,
    types::signing::{Principal, RecordDraft},
};
use syneroym_roym_core::{
    card::{MAILBOX_CARD_BODY_BYTES, card_body_upper_bound},
    clock,
    deal::{
        DealTerms,
        sd::{ConcealedTerms, reveal_terms_strict},
    },
    disclosure::{MAX_DETAILS_DISCLOSURE_BYTES, MAX_RECORD_DISCLOSURE_BYTES, Salt},
    envelope::Response,
    method::{self, DisclosedVerdict, DisclosedVerifyInput, Reply},
    paging,
    record::{Envelope, RECORD_QUOTE},
    transaction::{
        self, Answers, DealRole, EnquiryPayload, MAX_QUOTE_LIFETIME_SECS, MIN_QUOTE_LIFETIME_SECS,
        QUOTE_VERSION, QuotePayload, RecordVerdict,
    },
};

use super::{
    ENQUIRY_HISTORY, ListInput, OutgoingCard, QUOTE_HISTORY, QUOTES, RecordPointerRow, SendState,
    conversation_mine_filter, count_mine, decline_ops, disclosures, ensure_collections,
    extension_rules, get_bytes, get_row, hooks, is_deal_ended, put_bytes, put_row,
    resolve_principal_and_owner, send_card_and_file,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteSetInput {
    #[serde(alias = "request_record_id")]
    pub enquiry_record_id: String,
    #[serde(default)]
    pub quote_id: Option<String>,
    pub expires_in_secs: u64,
    pub terms: DealTermsInput,
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
pub struct QuoteMeasureInput {
    pub terms: DealTermsInput,
}

#[derive(Debug, Serialize)]
pub struct DetailMeasure {
    pub commitment_id: String,
    pub bytes: u64,
    pub limit: u64,
}

#[derive(Debug, Serialize)]
pub struct QuoteMeasureOutput {
    pub bytes: u64,
    pub limit: u64,
    pub details: Vec<DetailMeasure>,
    pub fits: bool,
    pub card_bytes: u64,
    pub mailbox_fits: bool,
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
    pub terms: DealTerms,
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
    pub terms: DealTerms,
}

#[derive(Debug, Serialize)]
pub struct QuoteListOutput {
    pub quotes: Vec<QuoteListItem>,
}

#[derive(Debug, Serialize)]
pub struct QuoteVersionItem {
    pub record_id: String,
    pub terms: DealTerms,
}

#[derive(Debug, Serialize)]
pub struct QuoteHistoryOutput {
    pub history: Vec<String>,
    pub versions: Vec<QuoteVersionItem>,
}

pub(crate) async fn prepare_terms<H: AppHost>(
    host: &H,
    input: &DealTermsInput,
    expires_at_secs: u64,
) -> Result<DealTerms, Response> {
    let mut terms_input = input.clone();
    let listing_id = terms_input.listing_id.clone();
    for c in &mut terms_input.commitments {
        if c.giver == DealRole::QuoteAuthor && hooks::has_prepare_hook(c.extension.as_ref()) {
            c.window = Some(hooks::prepare(host, c, listing_id.as_deref()).await?);
        }
    }
    Ok(terms_input.into_deal_terms(expires_at_secs))
}

pub(crate) async fn set_at<H: AppHost>(
    host: &H,
    mut input: QuoteSetInput,
    now: u64,
) -> Reply<QuoteSetOutput> {
    let (principal, owner) = resolve_principal_and_owner(host, now).await?;

    if !(MIN_QUOTE_LIFETIME_SECS..=MAX_QUOTE_LIFETIME_SECS).contains(&input.expires_in_secs) {
        return Err(Response::invalid_params(format!(
            "expires_in_secs must be between {MIN_QUOTE_LIFETIME_SECS} and \
             {MAX_QUOTE_LIFETIME_SECS}"
        )));
    }

    ensure_collections(host).await.map_err(Response::internal_error)?;

    let (req_payload, counterparty_did) =
        load_verified_enquiry(host, &input.enquiry_record_id, now, &owner).await?;
    let conversation = req_payload.conversation;

    let (sequence, supersedes, next_count) =
        resolve_quote_sequence(host, &input, &conversation, &owner, now).await?;

    let expires_at_secs = now + input.expires_in_secs;
    let quote_id = match input.quote_id.take() {
        Some(id) => id,
        None => match transaction::derive_quote_id(&conversation, &owner, sequence) {
            Ok(id) => id,
            Err(e) => return Err(Response::internal_error(e.to_string())),
        },
    };

    let terms = prepare_terms(host, &input.terms, expires_at_secs).await?;
    let concealed = validate_and_conceal_terms(host, &terms).await?;

    let payload = QuotePayload {
        quote_id: quote_id.clone(),
        conversation: conversation.clone(),
        sequence,
        answers: Answers::Enquiry { record_id: input.enquiry_record_id.clone() },
        counterparty_did: counterparty_did.clone(),
        terms: concealed.signed,
    };
    if let Err(e) = payload.validate() {
        let resp = Response::invalid_params(e.to_string());
        let resp = match &e {
            transaction::TransactionError::Deal(de) => {
                resp.with_data(serde_json::json!({ "code": de.code() }))
            }
            _ => resp,
        };
        return Err(resp);
    }

    let sign_input = QuoteSigningInput {
        principal,
        owner: &owner,
        expires_at_secs,
        supersedes,
        disclosures: &concealed.disclosures,
        now,
    };
    let signed = sign_and_store_quote(host, &payload, sign_input).await?;

    let pointer = build_quote_pointer(&payload, &signed, &owner, next_count, now);
    if let Err(e) = put_row(host, QUOTES, &quote_id, &pointer).await {
        return Err(Response::internal_error(e));
    }

    let card = OutgoingCard {
        card_type: RECORD_QUOTE,
        version: QUOTE_VERSION,
        envelope: &signed.envelope_json,
        send: &concealed.disclosures,
        view: &concealed.disclosures,
    };
    let (message_id, state, send_error) =
        send_card_and_file(host, &conversation, &card, now, Some(next_count)).await;

    Ok(QuoteSetOutput {
        quote_id,
        record_id: signed.record_id,
        version_count: next_count,
        expires_at_secs,
        message_id,
        state,
        send_error: send_error.map(|e| e.to_string()),
    })
}

pub(crate) async fn measure_at<H: AppHost>(
    host: &H,
    input: QuoteMeasureInput,
    now: u64,
) -> Reply<QuoteMeasureOutput> {
    let (principal, _owner) = resolve_principal_and_owner(host, now).await?;
    let terms = prepare_terms(host, &input.terms, now + MAX_QUOTE_LIFETIME_SECS).await?;

    let size = terms.disclosure_size().map_err(|e| {
        Response::invalid_params(e.to_string()).with_data(serde_json::json!({ "code": e.code() }))
    })?;

    let details: Vec<DetailMeasure> = size
        .details
        .into_iter()
        .map(|(id, bytes)| DetailMeasure {
            commitment_id: id,
            bytes: bytes as u64,
            limit: MAX_DETAILS_DISCLOSURE_BYTES as u64,
        })
        .collect();

    let fits = size.total_bytes <= MAX_RECORD_DISCLOSURE_BYTES
        && details.iter().all(|d| d.bytes <= d.limit);

    let mut zero_salts = (0..).map(|_| Salt::from_bytes([0u8; 16]));
    let concealed = terms.conceal_unbounded(&mut zero_salts).map_err(|e| {
        Response::invalid_params(e.to_string()).with_data(serde_json::json!({ "code": e.code() }))
    })?;

    let payload = QuotePayload {
        quote_id: "quo_0000000000000000000000000000000000000000000000000000".to_string(),
        conversation: "conv:0000000000000000000000000000000000000000000000000000000000000000"
            .to_string(),
        sequence: 1,
        answers: Answers::Enquiry {
            record_id: "rec_0000000000000000000000000000000000000000000000000000".to_string(),
        },
        counterparty_did: "did:key:z0000000000000000000000000000000000000000000000000000000"
            .to_string(),
        terms: concealed.signed,
    };

    let payload_val = match serde_json::to_value(&payload) {
        Ok(v) => v,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let cert_str = match &principal {
        Principal::Delegated(cert) => cert.as_str(),
        Principal::Service => "",
    };

    let card_bytes =
        card_body_upper_bound(RECORD_QUOTE, &payload_val, &concealed.disclosures, cert_str) as u64;
    let mailbox_fits = card_bytes <= MAILBOX_CARD_BODY_BYTES as u64;

    Ok(QuoteMeasureOutput {
        bytes: size.total_bytes as u64,
        limit: MAX_RECORD_DISCLOSURE_BYTES as u64,
        details,
        fits,
        card_bytes,
        mailbox_fits,
    })
}

pub(crate) async fn measure<H: AppHost>(
    host: &H,
    input: QuoteMeasureInput,
) -> Reply<QuoteMeasureOutput> {
    measure_at(host, input, clock::now_secs()).await
}

pub(crate) async fn set<H: AppHost>(host: &H, input: QuoteSetInput) -> Reply<QuoteSetOutput> {
    set_at(host, input, clock::now_secs()).await
}

/// Fetches and verifies the enquiry this quote answers, and checks the
/// caller is not the same party that made the enquiry.
async fn load_verified_enquiry<H: AppHost>(
    host: &H,
    enquiry_record_id: &str,
    now: u64,
    owner: &str,
) -> Result<(EnquiryPayload, String), Response> {
    let req_envelope_bytes = match get_bytes(host, ENQUIRY_HISTORY, enquiry_record_id).await {
        Ok(Some(b)) => b,
        Ok(None) => return Err(Response::invalid_params("no such enquiry")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let req_envelope_str = match String::from_utf8(req_envelope_bytes) {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let req_verdict = transaction::verify_enquiry(&req_envelope_str, now);
    if !req_verdict.verified {
        return Err(Response::invalid_params(format!(
            "the enquiry this quote answers does not verify: {}",
            req_verdict.reason.as_deref().unwrap_or("unknown")
        )));
    }
    let req_payload = match req_verdict.payload {
        Some(p) => p,
        None => return Err(Response::internal_error("missing enquiry payload")),
    };
    let counterparty_did = match req_verdict.issuer {
        Some(i) => i,
        None => return Err(Response::internal_error("missing enquiry issuer")),
    };
    if counterparty_did == owner {
        return Err(Response::invalid_params(
            "an enquiry cannot be quoted by the person who made it",
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
    now: u64,
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
        if prior.request_record_id.as_deref() != Some(&params.enquiry_record_id) {
            return Err(Response::invalid_params(
                "a new version of a quote answers the same enquiry",
            ));
        }

        if decline_ops::has_unrefused_counterparty_half(host, id)
            .await
            .map_err(Response::internal_error)?
        {
            return Err(Response::invalid_params("quote-already-accepted")
                .with_data(serde_json::json!("quote-already-accepted")));
        }

        if is_deal_ended(host, id, None, now).await.map_err(Response::internal_error)? {
            return Err(
                Response::invalid_params("quote-ended").with_data(serde_json::json!("quote-ended"))
            );
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

/// A freshly signed quote envelope, plus the identifiers callers need to
/// reference it: kept as a struct rather than a `(String, String)` tuple so
/// the two values can't be swapped at a call site.
struct SignedQuote {
    record_id: String,
    envelope_json: String,
}

async fn validate_and_conceal_terms<H: AppHost>(
    host: &H,
    terms: &DealTerms,
) -> Result<ConcealedTerms, Response> {
    if let Err(e) = terms.validate() {
        return Err(Response::invalid_params(e.to_string())
            .with_data(serde_json::json!({ "code": e.code() })));
    }
    extension_rules::require_known_extensions(terms)?;
    let salts = disclosures::fresh_salts(host, terms.hidden_field_count() as u32).await?;
    let mut salts_iter = salts.into_iter();
    terms.conceal(&mut salts_iter).map_err(|e| {
        Response::invalid_params(e.to_string()).with_data(serde_json::json!({ "code": e.code() }))
    })
}

struct QuoteSigningInput<'a> {
    principal: Principal,
    owner: &'a str,
    expires_at_secs: u64,
    supersedes: Option<String>,
    disclosures: &'a [String],
    now: u64,
}

/// Serializes and signs `payload`, checks the host signed it under the
/// issuer this service asked for, and stores the envelope by its record id.
async fn sign_and_store_quote<H: AppHost>(
    host: &H,
    payload: &QuotePayload,
    input: QuoteSigningInput<'_>,
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
        expires_at_secs: Some(input.expires_at_secs),
        supersedes: input.supersedes,
    };

    let envelope_json = match AppSigning::sign_record(host, draft, input.principal).await {
        Ok(json) => json,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let envelope = match Envelope::from_json(&envelope_json) {
        Ok(env) => env,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    if envelope.issuer != input.owner {
        return Err(Response::internal_error(
            "the host signed under an issuer this service did not ask for",
        ));
    }

    let record_id = match envelope.record_id() {
        Ok(id) => id,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    if let Err(e) =
        disclosures::store(host, &record_id, RECORD_QUOTE, input.disclosures, input.now).await
    {
        return Err(Response::internal_error(e));
    }

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
    let request_record_id = match &payload.answers {
        Answers::Enquiry { record_id } => Some(record_id.clone()),
        _ => None,
    };
    RecordPointerRow {
        formed_as: None,
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
        request_record_id,
        counterparty_did: Some(payload.counterparty_did.clone()),
        declined_at_secs: None,
        decline_note: None,
    }
}

async fn reveal_pointer_terms<H: AppHost>(
    host: &H,
    record_id: &str,
    envelope_str: &str,
) -> Result<DealTerms, String> {
    let env = Envelope::from_json(envelope_str).map_err(|e| e.to_string())?;
    let payload: QuotePayload = serde_json::from_value(env.payload).map_err(|e| e.to_string())?;
    let disc = disclosures::load(host, record_id)
        .await?
        .ok_or_else(|| "disclosures missing".to_string())?;
    reveal_terms_strict(&payload.terms, &disc).map_err(|e| e.to_string())
}

pub(crate) async fn get<H: AppHost>(host: &H, input: QuoteKeyInput) -> Reply<Option<QuoteView>> {
    ensure_collections(host).await.map_err(Response::internal_error)?;
    let pointer: Option<RecordPointerRow> = match get_row(host, QUOTES, &input.quote_id).await {
        Ok(p) => p,
        Err(e) => return Err(Response::internal_error(e)),
    };
    let pointer = match pointer {
        Some(p) => p,
        None => return Ok(None),
    };
    let terms = reveal_pointer_terms(host, &pointer.record_id, &pointer.envelope)
        .await
        .map_err(Response::internal_error)?;

    Ok(Some(QuoteView {
        envelope: pointer.envelope,
        record_id: pointer.record_id,
        quote_id: pointer.id,
        conversation: pointer.conversation,
        mine: pointer.mine,
        updated_at_secs: pointer.updated_at_secs,
        version_count: pointer.version_count,
        request_record_id: pointer.request_record_id,
        counterparty_did: pointer.counterparty_did,
        declined_at_secs: pointer.declined_at_secs,
        decline_note: pointer.decline_note,
        terms,
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
    let mut page: Vec<QuoteListItem> = Vec::with_capacity(limit.min(rows.len()));
    for p in rows.into_iter().skip(offset).take(limit) {
        let terms = reveal_pointer_terms(host, &p.record_id, &p.envelope)
            .await
            .map_err(Response::internal_error)?;
        page.push(QuoteListItem {
            envelope: p.envelope,
            record_id: p.record_id,
            quote_id: p.id,
            conversation: p.conversation,
            mine: p.mine,
            updated_at_secs: p.updated_at_secs,
            version_count: p.version_count,
            request_record_id: p.request_record_id,
            counterparty_did: p.counterparty_did,
            terms,
        });
    }
    Ok(QuoteListOutput { quotes: page })
}

pub(crate) async fn history<H: AppHost>(
    host: &H,
    input: QuoteKeyInput,
) -> Reply<QuoteHistoryOutput> {
    ensure_collections(host).await.map_err(Response::internal_error)?;
    let out = match paging::envelope_history(host, QUOTE_HISTORY, "quote_id", &input.quote_id).await
    {
        Ok(out) => out,
        Err(e) => return Err(Response::internal_error(e)),
    };
    let mut versions = Vec::with_capacity(out.len());
    for env_str in &out {
        if let Ok(env) = Envelope::from_json(env_str)
            && let Ok(rid) = env.record_id()
            && let Ok(terms) = reveal_pointer_terms(host, &rid, env_str).await
        {
            versions.push(QuoteVersionItem { record_id: rid, terms });
        }
    }
    Ok(QuoteHistoryOutput { history: out, versions })
}

pub(crate) async fn verify<H: AppHost>(
    _host: &H,
    input: DisclosedVerifyInput,
) -> Reply<DisclosedVerdict<QuotePayload>> {
    let now = clock::now_secs();
    let verdict = transaction::verify_quote(&input.envelope, now);
    Ok(method::verify_disclosed(verdict, &input.disclosures, |p, d| {
        method::reveal_disclosed_terms(&p.terms, d)
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
        let count = if existing.record_id == record_id {
            existing.version_count
        } else {
            existing.version_count + 1
        };
        (count, existing.declined_at_secs, existing.decline_note.clone())
    } else {
        (1, None, None)
    };
    let mine = !owner.is_empty() && v.issuer.as_deref() == Some(owner);
    let request_record_id = match &payload.answers {
        Answers::Enquiry { record_id } => Some(record_id.clone()),
        _ => None,
    };
    let pointer = RecordPointerRow {
        envelope: envelope.to_string(),
        formed_as: None,
        record_id: record_id.to_string(),
        id: payload.quote_id.clone(),
        conversation: payload.conversation.clone(),
        sequence: payload.sequence,
        issuer: v.issuer.clone().unwrap_or_default(),
        mine,
        updated_at_secs: now,
        version_count,
        issued_at_secs: incoming_issued,
        request_record_id,
        counterparty_did: Some(payload.counterparty_did.clone()),
        declined_at_secs,
        decline_note,
    };
    put_row(host, QUOTES, &payload.quote_id, &pointer).await?;
    Ok(version_count)
}
