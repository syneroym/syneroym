//! Agreement record operations: accept, get, list, verify.

use std::cmp::Reverse;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_app_host::{
    AppHost, AppSigning,
    types::signing::{Principal, RecordDraft},
};
use syneroym_roym_core::{
    clock,
    deal::{Commitment, DealTerms, ResourceKind, SignedTerms, transition::track_end},
    envelope::Response,
    lang::LangTag,
    method::{self, DisclosedVerdict, DisclosedVerifyInput, ParamError, Reply},
    paging,
    progress::{AgreementState, CancelReason, CommitmentProgress},
    record::{Envelope, RECORD_AGREEMENT_RECEIPT},
    signing,
    transaction::{
        self, AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, DealRole, PairState,
        QuotePayload, ReceiptHalf, pair_state,
    },
};

use super::{
    AGREEMENTS, AgreementRow, OutgoingCard, QUOTES, RecordPointerRow, SendState,
    default_list_limit_u64, disclosures, ensure_collections, get_row,
    hold::{Reservations, ReserveOutcome, SeatLedger},
    is_deal_ended, progress_ops, put_row, resolve_principal_and_owner, send_card_and_file,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteRecordInput {
    #[serde(alias = "agreement")]
    pub quote_record_id: String,
}

/// What a party read before it accepted. Both fields are optional: a caller
/// that shows the original says nothing, and the signed record then says the
/// quote's own language and that it was not a machine translation.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgreementAcceptInput {
    pub quote_record_id: String,
    #[serde(default)]
    pub viewed_lang: Option<String>,
    #[serde(default)]
    pub viewed_via_mt: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct AgreementAcceptOutput {
    pub quote_record_id: String,
    pub role: DealRole,
    pub record_id: String,
    pub pair: PairState,
    pub message_id: String,
    pub state: SendState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AgreementView {
    pub quote_record_id: String,
    pub quote_id: String,
    pub conversation: String,
    pub counterparty_did: String,
    pub quote_author_did: String,
    pub counterparty: Option<ReceiptHalf>,
    pub quote_author: Option<ReceiptHalf>,
    pub pair: PairState,
    pub terms: DealTerms,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<AgreementState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_reason: Option<CancelReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u32>,
    pub commitments: Vec<CommitmentProgress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agreed_at_secs: Option<u64>,
    pub track_ends: Vec<u64>,
}

impl From<AgreementRow> for AgreementView {
    fn from(r: AgreementRow) -> Self {
        let pair = pair_state(r.counterparty.as_ref(), r.quote_author.as_ref());
        let track_ends = if let Some(agreed_at) = r.agreed_at_secs {
            r.terms.commitments.iter().map(|c| track_end(c.window.as_ref(), agreed_at)).collect()
        } else {
            Vec::new()
        };
        Self {
            quote_record_id: r.quote_record_id,
            quote_id: r.quote_id,
            conversation: r.conversation,
            counterparty_did: r.counterparty_did,
            quote_author_did: r.quote_author_did,
            counterparty: r.counterparty,
            quote_author: r.quote_author,
            pair,
            terms: r.terms,
            state: r.state,
            state_reason: r.state_reason,
            cancel_reason: r.cancel_reason,
            seq: r.seq,
            commitments: r.commitments,
            agreed_at_secs: r.agreed_at_secs,
            track_ends,
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

pub(crate) async fn accept_at<H: AppHost>(
    host: &H,
    input: AgreementAcceptInput,
    now: u64,
) -> Reply<AgreementAcceptOutput> {
    let (principal, owner) = resolve_principal_and_owner(host, now).await?;
    ensure_collections(host).await.map_err(Response::internal_error)?;

    let (held, quote_author_did, role) =
        load_verified_quote_and_role(host, &input.quote_record_id, now, &owner).await?;

    if role == DealRole::QuoteAuthor {
        return Err(Response::invalid_params("writer-countersigns")
            .with_data(serde_json::json!("writer-countersigns")));
    }

    let viewed = viewed(&input, &held.terms)?;

    let mut row = load_or_init_agreement_row(
        host,
        &input.quote_record_id,
        &held.signed,
        &held.terms,
        &quote_author_did,
        now,
    )
    .await?;

    if let Some(out) = already_accepted_response(&row, role, &input.quote_record_id) {
        return Ok(out);
    }

    check_quote_acceptance_eligibility(
        host,
        &held.signed.quote_id,
        &input.quote_record_id,
        held.signed.terms.quote_expires_at_secs,
        now,
    )
    .await?;

    reserve_counterparty_slots(host, &input.quote_record_id, &row.terms.commitments, now).await?;

    let half = sign_agreement_receipt(
        host,
        SignReceiptInput {
            principal,
            quote_record_id: &input.quote_record_id,
            row: &row,
            signed_terms: held.signed.terms.clone(),
            owner: &owner,
            role,
            viewed,
        },
    )
    .await?;

    let envelope_json = half.envelope.clone();
    let record_id = half.record_id.clone();
    row.set_half(role, half);
    row.updated_at_secs = now;
    if let Err(e) = put_row(host, AGREEMENTS, &input.quote_record_id, &row).await {
        return Err(Response::internal_error(e));
    }

    let card = OutgoingCard {
        card_type: RECORD_AGREEMENT_RECEIPT,
        version: AGREEMENT_RECEIPT_VERSION,
        envelope: &envelope_json,
        send: &[],
        view: &held.disclosures,
    };

    let (message_id, state, send_error) =
        send_card_and_file(host, &held.signed.conversation, &card, now, None).await;

    let pair = pair_state(row.counterparty.as_ref(), row.quote_author.as_ref());
    Ok(AgreementAcceptOutput {
        quote_record_id: input.quote_record_id,
        role,
        record_id,
        pair,
        message_id,
        state,
        send_error: send_error.map(|e| e.to_string()),
    })
}

pub(crate) async fn accept<H: AppHost>(
    host: &H,
    input: AgreementAcceptInput,
) -> Reply<AgreementAcceptOutput> {
    accept_at(host, input, clock::now_secs()).await
}

async fn check_quote_acceptance_eligibility<H: AppHost>(
    host: &H,
    quote_id: &str,
    quote_record_id: &str,
    expires_at_secs: u64,
    now: u64,
) -> Result<(), Response> {
    if let Some(ptr) = get_row::<RecordPointerRow, _>(host, QUOTES, quote_id)
        .await
        .map_err(Response::internal_error)?
        && ptr.record_id != quote_record_id
    {
        return Err(Response::invalid_params("quote-superseded")
            .with_data(serde_json::json!("quote-superseded")));
    }

    if is_deal_ended(host, quote_id, Some(expires_at_secs), now)
        .await
        .map_err(Response::internal_error)?
    {
        return Err(
            Response::invalid_params("quote-ended").with_data(serde_json::json!("quote-ended"))
        );
    }
    Ok(())
}

async fn reserve_counterparty_slots<H: AppHost>(
    host: &H,
    quote_record_id: &str,
    commitments: &[Commitment],
    now: u64,
) -> Result<(), Response> {
    let reservations = SeatLedger;
    let mut reserved = Vec::new();
    for c in commitments {
        if c.giver == DealRole::Counterparty && c.resource_kind == ResourceKind::ServiceInSlot {
            let slot_id = c
                .details
                .as_ref()
                .and_then(|d| d.data.get("slot_id"))
                .and_then(Value::as_str)
                .ok_or_else(|| Response::invalid_params("missing slot_id"))?;
            match reservations.reserve(host, quote_record_id, &c.commitment_id, slot_id, now).await
            {
                Ok(ReserveOutcome::Held { .. } | ReserveOutcome::AlreadyHeld { .. }) => {
                    reserved.push(c.commitment_id.clone());
                }
                Ok(outcome @ (ReserveOutcome::SlotUnavailable | ReserveOutcome::SlotTaken)) => {
                    for cid in &reserved {
                        let _ = reservations.release(host, quote_record_id, cid).await;
                    }
                    let reason = if outcome == ReserveOutcome::SlotUnavailable {
                        "slot-unavailable"
                    } else {
                        "slot-taken"
                    };
                    return Err(Response::invalid_params("reserve-failed")
                        .with_data(serde_json::json!({ "reason": reason })));
                }
                Err(e) => {
                    for cid in &reserved {
                        let _ = reservations.release(host, quote_record_id, cid).await;
                    }
                    return Err(Response::internal_error(e));
                }
            }
        }
    }
    Ok(())
}

fn viewed_error(field: &str, message: impl Into<String>) -> Response {
    ParamError { field: Some(field.to_string()), message: message.into() }.into_response()
}

fn viewed(input: &AgreementAcceptInput, terms: &DealTerms) -> Result<(LangTag, bool), Response> {
    let original = &terms.scope.lang;
    let via_mt = input.viewed_via_mt.unwrap_or(false);
    let lang = match input.viewed_lang.as_deref() {
        None => original.clone(),
        Some(text) => LangTag::parse_canonical(text).map_err(|e| {
            viewed_error("viewed_lang", format!("invalid params: viewed_lang: {e}"))
        })?,
    };
    if let Err(e) = transaction::check_viewed(&lang, via_mt, terms) {
        return Err(viewed_error("viewed_via_mt", e.to_string()));
    }
    if !via_mt && lang != *original && !terms.scope.translations.iter().any(|t| t.lang == lang) {
        return Err(viewed_error("viewed_lang", "viewed_lang names no version of this quote"));
    }
    Ok((lang, via_mt))
}

async fn load_verified_quote_and_role<H: AppHost>(
    host: &H,
    quote_record_id: &str,
    now: u64,
    owner: &str,
) -> Result<(disclosures::HeldQuote, String, DealRole), Response> {
    let held = match disclosures::load_quote(host, quote_record_id, now).await {
        Ok(h) => h,
        Err(disclosures::LoadQuoteError::Missing) => {
            return Err(Response::invalid_params("no such quote"));
        }
        Err(disclosures::LoadQuoteError::Storage(e)) => return Err(Response::internal_error(e)),
        Err(disclosures::LoadQuoteError::Invalid(e)) => {
            return Err(Response::invalid_params(format!("the quote does not verify: {e}")));
        }
    };
    if held.verdict.expired {
        return Err(Response::invalid_params("quote-expired"));
    }

    let quote_author_did = match held.verdict.issuer.as_deref() {
        Some(i) => i.to_string(),
        None => return Err(Response::internal_error("missing quote issuer")),
    };

    let role = if owner == held.signed.counterparty_did {
        DealRole::Counterparty
    } else if owner == quote_author_did {
        DealRole::QuoteAuthor
    } else {
        return Err(Response::invalid_params("this installation is neither party to that quote"));
    };

    Ok((held, quote_author_did, role))
}

async fn load_or_init_agreement_row<H: AppHost>(
    host: &H,
    quote_record_id: &str,
    quote_payload: &QuotePayload,
    terms: &DealTerms,
    quote_author_did: &str,
    now: u64,
) -> Result<AgreementRow, Response> {
    match get_row(host, AGREEMENTS, quote_record_id).await {
        Ok(Some(r)) => Ok(r),
        Ok(None) => Ok(AgreementRow {
            quote_record_id: quote_record_id.to_string(),
            quote_id: quote_payload.quote_id.clone(),
            conversation: quote_payload.conversation.clone(),
            counterparty_did: quote_payload.counterparty_did.clone(),
            quote_author_did: quote_author_did.to_string(),
            terms: terms.clone(),
            state: None,
            state_reason: None,
            cancel_reason: None,
            seq: None,
            commitments: Vec::new(),
            agreed_at_secs: None,
            counterparty: None,
            quote_author: None,
            updated_at_secs: now,
        }),
        Err(e) => Err(Response::internal_error(e)),
    }
}

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
        message_id: String::new(),
        state: SendState::AlreadyAccepted,
        send_error: None,
    })
}

struct SignReceiptInput<'a> {
    principal: Principal,
    quote_record_id: &'a str,
    row: &'a AgreementRow,
    signed_terms: SignedTerms,
    owner: &'a str,
    role: DealRole,
    viewed: (LangTag, bool),
}

async fn sign_agreement_receipt<H: AppHost>(
    host: &H,
    input: SignReceiptInput<'_>,
) -> Result<ReceiptHalf, Response> {
    let (viewed_lang, viewed_via_mt) = input.viewed;
    let payload = AgreementReceiptPayload {
        quote_record_id: input.quote_record_id.to_string(),
        counterparty_did: input.row.counterparty_did.clone(),
        quote_author_did: input.row.quote_author_did.clone(),
        role: input.role,
        terms: input.signed_terms,
        viewed_lang,
        viewed_via_mt,
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

    let payload_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let draft = RecordDraft {
        version: AGREEMENT_RECEIPT_VERSION,
        record_type: RECORD_AGREEMENT_RECEIPT.to_string(),
        subject: input.quote_record_id.to_string(),
        payload: payload_str,
        expires_at_secs: None,
        supersedes: None,
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

    Ok(ReceiptHalf {
        envelope: envelope_json,
        record_id,
        issuer: input.owner.to_string(),
        issued_at_secs: envelope.issued_at_secs,
    })
}

pub(crate) async fn get_at<H: AppHost>(
    host: &H,
    input: QuoteRecordInput,
    now: u64,
) -> Reply<Option<AgreementView>> {
    ensure_collections(host).await.map_err(Response::internal_error)?;
    let mut row: Option<AgreementRow> =
        match get_row(host, AGREEMENTS, &input.quote_record_id).await {
            Ok(r) => r,
            Err(e) => return Err(Response::internal_error(e)),
        };
    let Some(ref mut agr) = row else {
        return Ok(None);
    };

    if let Ok(owner) = signing::owner_did(host).await {
        if owner == agr.quote_author_did {
            let _ = progress_ops::tick_agreement_at(host, agr, now).await;
        } else if owner == agr.counterparty_did
            && agr.quote_author.is_none()
            && now >= agr.terms.quote_expires_at_secs
        {
            let reservations = SeatLedger;
            for c in &agr.terms.commitments {
                if c.giver == DealRole::Counterparty
                    && c.resource_kind == ResourceKind::ServiceInSlot
                {
                    let _ =
                        reservations.release(host, &agr.quote_record_id, &c.commitment_id).await;
                }
            }
        }
    }

    Ok(Some(AgreementView::from(agr.clone())))
}

pub(crate) async fn get<H: AppHost>(
    host: &H,
    input: QuoteRecordInput,
) -> Reply<Option<AgreementView>> {
    get_at(host, input, clock::now_secs()).await
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
    input: DisclosedVerifyInput,
) -> Reply<DisclosedVerdict<AgreementReceiptPayload>> {
    let now = clock::now_secs();
    let verdict = transaction::verify_agreement_receipt(&input.envelope, now);
    let (shown, hidden, disclosure_refused) = if verdict.verified {
        if let Some(ref p) = verdict.payload {
            method::reveal_disclosed_terms(&p.terms, &input.disclosures)
        } else {
            (None, Vec::new(), None)
        }
    } else {
        (None, Vec::new(), None)
    };
    Ok(DisclosedVerdict { verdict, shown, hidden, disclosure_refused })
}
