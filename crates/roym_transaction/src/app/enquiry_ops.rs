//! Enquiry record operations: set, get, list, history.

use std::cmp::Reverse;

use serde::{Deserialize, Serialize};
use syneroym_app_host::{
    AppHost, AppSigning,
    types::signing::{Principal, RecordDraft},
};
use syneroym_roym_core::{
    area::Area,
    clock,
    deal::LocalText,
    envelope::Response,
    method::{self, EnvelopeHistoryOutput, Reply, VerifyInput},
    paging,
    record::{Envelope, RECORD_ENQUIRY},
    transaction::{self, ENQUIRY_VERSION, EnquiryPayload, RecordVerdict, TimeWindow},
};

use super::{
    ENQUIRIES, ENQUIRY_HISTORY, ListInput, RecordPointerRow, SendState, conversation_mine_filter,
    count_mine, ensure_collections, get_row, put_bytes, put_row, resolve_principal_and_owner,
    send_card_and_file,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnquirySetInput {
    pub conversation: String,
    #[serde(default)]
    pub enquiry_id: Option<String>,
    #[serde(default)]
    pub listing_id: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    pub description: LocalText,
    #[serde(default)]
    pub area: Option<Area>,
    #[serde(default)]
    pub window: Option<TimeWindow>,
    pub data_use_notice: String,
}

#[derive(Debug, Serialize)]
pub struct EnquirySetOutput {
    pub enquiry_id: String,
    pub record_id: String,
    pub version_count: u64,
    pub message_id: String,
    pub state: SendState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnquiryKeyInput {
    pub enquiry_id: String,
}

#[derive(Debug, Serialize)]
pub struct EnquiryView {
    pub envelope: String,
    pub record_id: String,
    pub enquiry_id: String,
    pub conversation: String,
    pub mine: bool,
    pub updated_at_secs: u64,
    pub version_count: u64,
    pub declined_at_secs: Option<u64>,
    pub decline_note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EnquiryListOutput {
    pub enquiries: Vec<EnquiryView>,
}

pub(crate) async fn set<H: AppHost>(host: &H, input: EnquirySetInput) -> Reply<EnquirySetOutput> {
    let now = clock::now_secs();
    let (principal, owner) = resolve_principal_and_owner(host, now).await?;

    ensure_collections(host).await.map_err(Response::internal_error)?;

    let (sequence, supersedes, next_count) = resolve_enquiry_sequence(host, &input, &owner).await?;

    let enquiry_id = match transaction::derive_enquiry_id(&input.conversation, &owner, sequence) {
        Ok(id) => id,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let payload = EnquiryPayload {
        enquiry_id: enquiry_id.clone(),
        conversation: input.conversation.clone(),
        sequence,
        listing_id: input.listing_id,
        categories: input.categories,
        description: input.description,
        area: input.area,
        window: input.window,
        data_use_notice: input.data_use_notice,
    };
    if let Err(e) = payload.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }

    let signed = sign_and_store_enquiry(host, principal, &owner, &payload, supersedes).await?;

    let pointer = build_enquiry_pointer(&payload, &signed, &owner, next_count, now);
    if let Err(e) = put_row(host, ENQUIRIES, &enquiry_id, &pointer).await {
        return Err(Response::internal_error(e));
    }

    let (message_id, state, send_error) = send_card_and_file(
        host,
        &input.conversation,
        RECORD_ENQUIRY,
        ENQUIRY_VERSION,
        &signed.envelope_json,
        now,
        Some(next_count),
    )
    .await;

    Ok(EnquirySetOutput {
        enquiry_id,
        record_id: signed.record_id,
        version_count: next_count,
        message_id,
        state,
        send_error: send_error.map(|e| e.to_string()),
    })
}

/// Works out this enquiry's sequence number and, when it revises an earlier
/// enquiry, which record it supersedes.
async fn resolve_enquiry_sequence<H: AppHost>(
    host: &H,
    params: &EnquirySetInput,
    owner: &str,
) -> Result<(u32, Option<String>, u64), Response> {
    if let Some(ref id) = params.enquiry_id {
        let prior: RecordPointerRow = match get_row(host, ENQUIRIES, id).await {
            Ok(Some(p)) => p,
            Ok(None) => return Err(Response::invalid_params("no such enquiry")),
            Err(e) => return Err(Response::internal_error(e)),
        };
        if !prior.mine {
            return Err(Response::invalid_params("this enquiry is not yours to revise"));
        }
        if prior.conversation != params.conversation {
            return Err(Response::invalid_params("enquiry belongs to another conversation"));
        }
        Ok((prior.sequence, Some(prior.record_id), prior.version_count + 1))
    } else {
        let count = match count_mine(host, ENQUIRIES, &params.conversation, owner).await {
            Ok(c) => c,
            Err(e) => return Err(Response::internal_error(e)),
        };
        Ok((count + 1, None, 1))
    }
}

/// A freshly signed enquiry envelope, plus the identifiers callers need to
/// reference it: kept as a struct rather than a `(String, String)` tuple so
/// the two values can't be swapped at a call site.
struct SignedEnquiry {
    record_id: String,
    envelope_json: String,
}

/// Serializes and signs `payload`, checks the host signed it under the
/// issuer this service asked for, and stores the envelope by its record id.
async fn sign_and_store_enquiry<H: AppHost>(
    host: &H,
    principal: Principal,
    owner: &str,
    payload: &EnquiryPayload,
    supersedes: Option<String>,
) -> Result<SignedEnquiry, Response> {
    let payload_str = match serde_json::to_string(payload) {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let draft = RecordDraft {
        version: ENQUIRY_VERSION,
        record_type: RECORD_ENQUIRY.to_string(),
        subject: payload.enquiry_id.clone(),
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
    if let Err(e) = put_bytes(host, ENQUIRY_HISTORY, &record_id, envelope_json.as_bytes()).await {
        return Err(Response::internal_error(e));
    }

    Ok(SignedEnquiry { record_id, envelope_json })
}

/// Builds the pointer row this node keeps to find the enquiry again,
/// deriving the identity fields from `payload` so they can't drift from the
/// record that was actually signed.
fn build_enquiry_pointer(
    payload: &EnquiryPayload,
    signed: &SignedEnquiry,
    owner: &str,
    next_count: u64,
    now: u64,
) -> RecordPointerRow {
    RecordPointerRow {
        envelope: signed.envelope_json.clone(),
        record_id: signed.record_id.clone(),
        id: payload.enquiry_id.clone(),
        conversation: payload.conversation.clone(),
        sequence: payload.sequence,
        issuer: owner.to_string(),
        mine: true,
        updated_at_secs: now,
        version_count: next_count,
        issued_at_secs: now,
        request_record_id: None,
        counterparty_did: None,
        declined_at_secs: None,
        decline_note: None,
    }
}

pub(crate) async fn get<H: AppHost>(
    host: &H,
    input: EnquiryKeyInput,
) -> Reply<Option<EnquiryView>> {
    ensure_collections(host).await.map_err(Response::internal_error)?;
    let pointer: Option<RecordPointerRow> = match get_row(host, ENQUIRIES, &input.enquiry_id).await
    {
        Ok(p) => p,
        Err(e) => return Err(Response::internal_error(e)),
    };
    Ok(pointer.map(|p| EnquiryView {
        envelope: p.envelope,
        record_id: p.record_id,
        enquiry_id: p.id,
        conversation: p.conversation,
        mine: p.mine,
        updated_at_secs: p.updated_at_secs,
        version_count: p.version_count,
        declined_at_secs: p.declined_at_secs,
        decline_note: p.decline_note,
    }))
}

pub(crate) async fn list<H: AppHost>(host: &H, input: ListInput) -> Reply<EnquiryListOutput> {
    ensure_collections(host).await.map_err(Response::internal_error)?;

    let filter = conversation_mine_filter(input.conversation.as_deref(), input.mine);
    let mut rows: Vec<RecordPointerRow> = match paging::query_all(host, ENQUIRIES, filter).await {
        Ok(rows) => rows,
        Err(e) => return Err(Response::internal_error(e)),
    };
    rows.sort_by_key(|p| Reverse(p.updated_at_secs));
    let offset = usize::try_from(input.offset).unwrap_or(usize::MAX);
    let limit = usize::try_from(input.limit).unwrap_or(usize::MAX);
    let page: Vec<EnquiryView> = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|p| EnquiryView {
            envelope: p.envelope,
            record_id: p.record_id,
            enquiry_id: p.id,
            conversation: p.conversation,
            mine: p.mine,
            updated_at_secs: p.updated_at_secs,
            version_count: p.version_count,
            declined_at_secs: p.declined_at_secs,
            decline_note: p.decline_note,
        })
        .collect();
    Ok(EnquiryListOutput { enquiries: page })
}

pub(crate) async fn history<H: AppHost>(
    host: &H,
    input: EnquiryKeyInput,
) -> Reply<EnvelopeHistoryOutput> {
    ensure_collections(host).await.map_err(Response::internal_error)?;
    let out = match paging::envelope_history(host, ENQUIRY_HISTORY, "enquiry_id", &input.enquiry_id)
        .await
    {
        Ok(out) => out,
        Err(e) => return Err(Response::internal_error(e)),
    };
    Ok(EnvelopeHistoryOutput { history: out })
}

pub(crate) async fn verify<H: AppHost>(
    _host: &H,
    input: VerifyInput,
) -> Reply<RecordVerdict<EnquiryPayload>> {
    method::verify_with(input, transaction::verify_enquiry)
}

pub(crate) async fn store_received_enquiry<H: AppHost>(
    host: &H,
    envelope: &str,
    v: &RecordVerdict<EnquiryPayload>,
    payload: &EnquiryPayload,
    now: u64,
    owner: &str,
) -> Result<u64, String> {
    let record_id = v.record_id.as_deref().unwrap_or_default();
    let prior: Option<RecordPointerRow> = get_row(host, ENQUIRIES, &payload.enquiry_id).await?;
    let incoming_issued = v.issued_at_secs.unwrap_or(now);
    if let Some(existing) = &prior
        && existing.record_id != record_id
        && incoming_issued <= existing.issued_at_secs
        && v.supersedes.as_deref() != Some(existing.record_id.as_str())
    {
        return Err("a newer or equal version of this enquiry is already held here".to_string());
    }
    put_bytes(host, ENQUIRY_HISTORY, record_id, envelope.as_bytes()).await?;
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
    let counterparty_did = if !mine && !owner.is_empty() { Some(owner.to_string()) } else { None };
    let pointer = RecordPointerRow {
        envelope: envelope.to_string(),
        record_id: record_id.to_string(),
        id: payload.enquiry_id.clone(),
        conversation: payload.conversation.clone(),
        sequence: payload.sequence,
        issuer: v.issuer.clone().unwrap_or_default(),
        mine,
        updated_at_secs: now,
        version_count,
        issued_at_secs: incoming_issued,
        request_record_id: None,
        counterparty_did,
        declined_at_secs: None,
        decline_note: None,
    };
    put_row(host, ENQUIRIES, &payload.enquiry_id, &pointer).await?;
    Ok(version_count)
}
