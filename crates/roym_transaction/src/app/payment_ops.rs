//! Payment request, acknowledgement, retrieval, and verification operations.

mod fence;

use fence::{AckFenceClaim, claim_payment_ack_fence, claim_payment_request_fence};
use serde::{Deserialize, Serialize};
use syneroym_app_host::{
    AppHost, AppSigning,
    types::signing::{Principal, RecordDraft},
};
use syneroym_roym_core::{
    booking::{BookingEvent, BookingState, Track, TrackState},
    clock,
    envelope::Response,
    method::{Reply, VerifyInput},
    payment::{self, PaymentAcknowledgementPayload, PaymentRequestPayload},
    record::{Envelope, RECORD_PAYMENT_ACKNOWLEDGEMENT, RECORD_PAYMENT_REQUEST},
    transaction::{DealRole, PairState, ReceiptHalf, pair_state},
    verdict::RecordVerdict,
};

use super::{
    AGREEMENTS, AgreementRef, AgreementRow, BookingRow, FenceOutcome, PAYMENTS, PROGRESS,
    PaymentsRow, ProgressRow, SendState, booking_ops, get_row, ledger::load_booking, put_row,
    resolve_principal_and_owner, send_card_and_file,
};
pub use crate::app::HalfRecordedOutput;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentRequestInput {
    pub agreement: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentRequestOutput {
    pub record_id: String,
    pub message_id: Option<String>,
    pub state: SendState,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentAckInput {
    pub agreement: String,
    #[serde(default)]
    pub observed_at_secs: Option<u64>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub supersedes: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PaymentGetOutput {
    pub request: Option<ReceiptHalf>,
    pub counterparty: Vec<ReceiptHalf>,
    pub quote_author: Vec<ReceiptHalf>,
    pub track: TrackState,
}

pub(crate) async fn request<H: AppHost>(
    host: &H,
    input: PaymentRequestInput,
) -> Reply<PaymentRequestOutput> {
    let now = clock::now_secs();
    let (principal, owner) = resolve_principal_and_owner(host, now).await?;

    let agr: AgreementRow = match get_row(host, AGREEMENTS, &input.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(Response::invalid_params("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let booking = match load_booking(host, &input.agreement).await {
        Ok(Some(b)) => b,
        Ok(None) => return Err(Response::invalid_params("no-booking")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    validate_payment_request_preconditions(&agr, &booking, &owner)?;

    let mut payments: PaymentsRow = match get_row(host, PAYMENTS, &input.agreement).await {
        Ok(Some(py)) => py,
        Ok(None) => PaymentsRow {
            agreement: input.agreement.clone(),
            conversation: agr.conversation.clone(),
            request: None,
            counterparty: Vec::new(),
            quote_author: Vec::new(),
            updated_at_secs: now,
        },
        Err(e) => return Err(Response::internal_error(e)),
    };

    if let Some(existing) = &payments.request {
        return Ok(PaymentRequestOutput {
            record_id: existing.record_id.clone(),
            message_id: None,
            state: SendState::AlreadyRecorded,
        });
    }

    let payload = PaymentRequestPayload {
        agreement: input.agreement.clone(),
        conversation: agr.conversation.clone(),
        counterparty_did: agr.counterparty_did.clone(),
        quote_author_did: agr.quote_author_did.clone(),
        currency: agr.terms.currency.clone(),
        amount_minor: agr.terms.amount_minor,
        note: input.note,
    };
    if let Err(e) = payload.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }

    let (env_json, env, record_id) =
        sign_payment_request(host, &input.agreement, &payload, principal).await?;

    let half = ReceiptHalf {
        record_id: record_id.clone(),
        envelope: env_json.clone(),
        issuer: env.issuer,
        issued_at_secs: env.issued_at_secs,
    };

    match claim_payment_request_fence(host, &input.agreement, &agr.conversation, &half, now).await?
    {
        FenceOutcome::Claimed => {}
        FenceOutcome::Answered(out) => return Ok(out),
    }

    payments.request = Some(half);
    payments.updated_at_secs = now;
    if let Err(e) = put_row(host, PAYMENTS, &input.agreement, &payments).await {
        return Err(Response::internal_error(e));
    }

    let (message_id, send_state, _) =
        send_card_and_file(host, &agr.conversation, "payment-request", 1, &env_json, now, None)
            .await;

    Ok(PaymentRequestOutput { record_id, message_id: Some(message_id), state: send_state })
}

struct AckContext {
    principal: Principal,
    owner: String,
    agr: AgreementRow,
    role: DealRole,
    payments: PaymentsRow,
}

async fn load_ack_context<H: AppHost>(
    host: &H,
    agreement_id: &str,
    now: u64,
) -> Result<AckContext, Response> {
    let (principal, owner) = match resolve_principal_and_owner(host, now).await {
        Ok(res) => res,
        Err(resp) => return Err(resp),
    };
    let agr: AgreementRow = match get_row(host, AGREEMENTS, agreement_id).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(Response::invalid_params("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };
    let role = match check_acknowledgement_preconditions(host, agreement_id, &agr, &owner).await {
        Ok(r) => r,
        Err(resp) => return Err(resp),
    };
    let payments: PaymentsRow = match get_row(host, PAYMENTS, agreement_id).await {
        Ok(Some(py)) => py,
        Ok(None) => PaymentsRow {
            agreement: agreement_id.to_string(),
            conversation: agr.conversation.clone(),
            request: None,
            counterparty: Vec::new(),
            quote_author: Vec::new(),
            updated_at_secs: now,
        },
        Err(e) => return Err(Response::internal_error(e)),
    };
    Ok(AckContext { principal, owner, agr, role, payments })
}

async fn maybe_transition_author_ack<H: AppHost>(
    host: &H,
    agreement: &str,
    owner: &str,
    quote_author_did: &str,
    now: u64,
) {
    if owner == quote_author_did {
        let _ = booking_ops::transition(
            host,
            agreement,
            BookingEvent::Half { track: Track::Payment, role: DealRole::QuoteAuthor },
            now,
        )
        .await;
    }
}

pub(crate) async fn acknowledge<H: AppHost>(
    host: &H,
    input: PaymentAckInput,
) -> Reply<HalfRecordedOutput> {
    let now = clock::now_secs();
    let AckContext { principal, owner, agr, role, mut payments } =
        load_ack_context(host, &input.agreement, now).await?;

    let versions = match role {
        DealRole::Counterparty => &mut payments.counterparty,
        DealRole::QuoteAuthor => &mut payments.quote_author,
    };

    if let Some(early_out) =
        match check_acknowledgement_version(input.supersedes.as_deref(), versions, role) {
            Ok(opt) => opt,
            Err(resp) => return Err(resp),
        }
    {
        maybe_transition_author_ack(host, &input.agreement, &owner, &agr.quote_author_did, now)
            .await;
        return Ok(early_out);
    }

    let payload = build_payment_ack_payload(&input, &agr, role, now)?;

    check_acknowledgement_correction(input.supersedes.as_deref(), versions, &payload, now)?;

    let (env_json, env, record_id) =
        sign_payment_ack(host, &input.agreement, &payload, input.supersedes.clone(), principal)
            .await?;

    let half = ReceiptHalf {
        record_id: record_id.clone(),
        envelope: env_json,
        issuer: env.issuer,
        issued_at_secs: env.issued_at_secs,
    };

    let ack_fence_claim = AckFenceClaim {
        agreement: &input.agreement,
        conversation: &agr.conversation,
        role,
        supersedes: input.supersedes.as_deref(),
        owner: &owner,
        quote_author_did: &agr.quote_author_did,
    };
    match claim_payment_ack_fence(host, ack_fence_claim, &half, now).await? {
        FenceOutcome::Claimed => {}
        FenceOutcome::Answered(out) => return Ok(out),
    }

    let (message_id, send_state) = record_and_send_ack(
        host,
        &input.agreement,
        &agr.conversation,
        &mut payments,
        role,
        half,
        now,
    )
    .await?;

    maybe_transition_author_ack(host, &input.agreement, &owner, &agr.quote_author_did, now).await;

    Ok(HalfRecordedOutput { record_id, role, message_id: Some(message_id), state: send_state })
}

async fn check_booking_not_terminal<H: AppHost>(
    host: &H,
    agreement: &str,
    role: DealRole,
) -> Option<Response> {
    match role {
        DealRole::QuoteAuthor => {
            if let Ok(Some(b)) = load_booking(host, agreement).await {
                if b.state == BookingState::Conflict {
                    return Some(Response::invalid_params("booking-conflict"));
                }
                if b.state == BookingState::Cancelled {
                    return Some(Response::invalid_params("booking-cancelled"));
                }
            }
        }
        DealRole::Counterparty => {
            if let Ok(Some(pr)) = get_row::<ProgressRow, _>(host, PROGRESS, agreement).await {
                if pr.snapshot.state == BookingState::Conflict {
                    return Some(Response::invalid_params("booking-conflict"));
                }
                if pr.snapshot.state == BookingState::Cancelled {
                    return Some(Response::invalid_params("booking-cancelled"));
                }
            }
        }
    }
    None
}

fn validate_payment_request_preconditions(
    agr: &AgreementRow,
    booking: &BookingRow,
    owner: &str,
) -> Result<(), Response> {
    let pair = pair_state(agr.counterparty.as_ref(), agr.quote_author.as_ref());
    if pair != PairState::Complete {
        return Err(Response::invalid_params("agreement-incomplete"));
    }
    if owner != agr.quote_author_did {
        return Err(Response::invalid_params("quote-author-only"));
    }
    match booking.state {
        BookingState::Scheduled | BookingState::InProgress => {}
        BookingState::Conflict => return Err(Response::invalid_params("booking-conflict")),
        BookingState::Cancelled => return Err(Response::invalid_params("booking-cancelled")),
        BookingState::Completed => return Err(Response::invalid_params("booking-completed")),
        BookingState::EndedUnconfirmed => {
            return Err(Response::invalid_params("booking-ended-unconfirmed"));
        }
    }
    if booking.snapshot.payment == TrackState::Acknowledged {
        return Err(Response::invalid_params("booking-payment-acknowledged"));
    }
    if agr.terms.amount_minor == 0 {
        return Err(Response::invalid_params("nothing-to-pay"));
    }
    Ok(())
}

async fn check_acknowledgement_preconditions<H: AppHost>(
    host: &H,
    agreement: &str,
    agr: &AgreementRow,
    owner: &str,
) -> Result<DealRole, Response> {
    let pair = pair_state(agr.counterparty.as_ref(), agr.quote_author.as_ref());
    if pair != PairState::Complete {
        return Err(Response::invalid_params("agreement-incomplete"));
    }

    let role = if owner == agr.quote_author_did {
        DealRole::QuoteAuthor
    } else if owner == agr.counterparty_did {
        DealRole::Counterparty
    } else {
        return Err(Response::invalid_params("not-a-party"));
    };

    if let Some(err) = check_booking_not_terminal(host, agreement, role).await {
        return Err(err);
    }
    Ok(role)
}

pub(super) fn check_acknowledgement_version(
    supersedes: Option<&str>,
    versions: &[ReceiptHalf],
    role: DealRole,
) -> Result<Option<HalfRecordedOutput>, Response> {
    match supersedes {
        None if !versions.is_empty() => {
            let rec_id = versions.last().map(|v| v.record_id.as_str()).unwrap_or_default();
            Ok(Some(HalfRecordedOutput {
                record_id: rec_id.to_string(),
                role,
                message_id: None,
                state: SendState::AlreadyRecorded,
            }))
        }
        None => Ok(None),
        Some(prev) => {
            if versions.last().map(|v| &v.record_id) != Some(&prev.to_string()) {
                Err(Response::invalid_params("not-the-current-version"))
            } else {
                Ok(None)
            }
        }
    }
}

fn check_acknowledgement_correction(
    supersedes: Option<&str>,
    versions: &[ReceiptHalf],
    payload: &PaymentAcknowledgementPayload,
    now: u64,
) -> Result<(), Response> {
    if supersedes.is_some()
        && let Some(last_half) = versions.last()
    {
        let prev_v = payment::verify_payment_acknowledgement(&last_half.envelope, now);
        if let Some(prev_p) = prev_v.payload
            && !payment::is_valid_correction(&prev_p, payload)
        {
            return Err(Response::invalid_params("invalid-correction"));
        }
    }
    Ok(())
}

pub(crate) async fn get<H: AppHost>(host: &H, input: AgreementRef) -> Reply<PaymentGetOutput> {
    let _agr: AgreementRow = match get_row(host, AGREEMENTS, &input.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(Response::invalid_params("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let payments: PaymentsRow = match get_row(host, PAYMENTS, &input.agreement).await {
        Ok(Some(py)) => py,
        Ok(None) => PaymentsRow::default(),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let track = if let Ok(Some(b)) = load_booking(host, &input.agreement).await {
        b.snapshot.payment
    } else if let Ok(Some(pr)) = get_row::<ProgressRow, _>(host, PROGRESS, &input.agreement).await {
        pr.snapshot.payment
    } else if !payments.quote_author.is_empty() {
        TrackState::Acknowledged
    } else if !payments.counterparty.is_empty() {
        TrackState::Claimed
    } else {
        TrackState::None
    };

    Ok(PaymentGetOutput {
        request: payments.request,
        counterparty: payments.counterparty,
        quote_author: payments.quote_author,
        track,
    })
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum PaymentVerdict {
    Request(RecordVerdict<PaymentRequestPayload>),
    Ack(RecordVerdict<PaymentAcknowledgementPayload>),
    Refused(RecordVerdict<()>),
}

pub(crate) async fn verify<H: AppHost>(_host: &H, input: VerifyInput) -> Reply<PaymentVerdict> {
    let now = clock::now_secs();
    let envelope = match Envelope::from_json(&input.envelope) {
        Ok(e) => e,
        Err(e) => {
            return Ok(PaymentVerdict::Refused(RecordVerdict::<()>::refused(format!(
                "envelope is not valid json: {e}"
            ))));
        }
    };
    match envelope.record_type.as_str() {
        RECORD_PAYMENT_REQUEST => {
            Ok(PaymentVerdict::Request(payment::verify_payment_request(&input.envelope, now)))
        }
        RECORD_PAYMENT_ACKNOWLEDGEMENT => {
            Ok(PaymentVerdict::Ack(payment::verify_payment_acknowledgement(&input.envelope, now)))
        }
        _ => Ok(PaymentVerdict::Refused(RecordVerdict::<()>::refused("not a payment record type"))),
    }
}

fn build_payment_ack_payload(
    p: &PaymentAckInput,
    agr: &AgreementRow,
    role: DealRole,
    now: u64,
) -> Result<PaymentAcknowledgementPayload, Response> {
    if !payment::matches_terms(
        &agr.terms.currency,
        agr.terms.amount_minor,
        p.method.as_deref(),
        &agr.terms,
    ) {
        return Err(Response::invalid_params("method-not-in-terms"));
    }
    let payload = PaymentAcknowledgementPayload {
        agreement: p.agreement.clone(),
        conversation: agr.conversation.clone(),
        counterparty_did: agr.counterparty_did.clone(),
        quote_author_did: agr.quote_author_did.clone(),
        role,
        currency: agr.terms.currency.clone(),
        amount_minor: agr.terms.amount_minor,
        observed_at_secs: p.observed_at_secs.unwrap_or(now),
        method: p.method.clone(),
        reference: p.reference.clone(),
    };
    if let Err(e) = payload.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }
    Ok(payload)
}

async fn sign_payment_request<H: AppHost>(
    host: &H,
    agreement: &str,
    payload: &PaymentRequestPayload,
    principal: Principal,
) -> Result<(String, Envelope, String), Response> {
    let payload_str = match serde_json::to_string(payload) {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let draft = RecordDraft {
        version: payment::PAYMENT_REQUEST_VERSION,
        record_type: RECORD_PAYMENT_REQUEST.to_string(),
        subject: agreement.to_string(),
        payload: payload_str,
        expires_at_secs: None,
        supersedes: None,
    };
    let env_json = match AppSigning::sign_record(host, draft, principal).await {
        Ok(j) => j,
        Err(e) => return Err(Response::invalid_params(e.to_string())),
    };
    let env = match Envelope::from_json(&env_json) {
        Ok(e) => e,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let record_id = match env.record_id() {
        Ok(id) => id,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    Ok((env_json, env, record_id))
}

async fn sign_payment_ack<H: AppHost>(
    host: &H,
    agreement: &str,
    payload: &PaymentAcknowledgementPayload,
    supersedes: Option<String>,
    principal: Principal,
) -> Result<(String, Envelope, String), Response> {
    let payload_str = match serde_json::to_string(payload) {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let draft = RecordDraft {
        version: payment::PAYMENT_ACKNOWLEDGEMENT_VERSION,
        record_type: RECORD_PAYMENT_ACKNOWLEDGEMENT.to_string(),
        subject: agreement.to_string(),
        payload: payload_str,
        expires_at_secs: None,
        supersedes,
    };
    let env_json = match AppSigning::sign_record(host, draft, principal).await {
        Ok(j) => j,
        Err(e) => return Err(Response::invalid_params(e.to_string())),
    };
    let env = match Envelope::from_json(&env_json) {
        Ok(e) => e,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let record_id = match env.record_id() {
        Ok(id) => id,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    Ok((env_json, env, record_id))
}

async fn record_and_send_ack<H: AppHost>(
    host: &H,
    agreement: &str,
    conversation: &str,
    payments: &mut PaymentsRow,
    role: DealRole,
    half: ReceiptHalf,
    now: u64,
) -> Result<(String, SendState), Response> {
    let env_json = half.envelope.clone();
    let versions = match role {
        DealRole::Counterparty => &mut payments.counterparty,
        DealRole::QuoteAuthor => &mut payments.quote_author,
    };
    versions.push(half);
    payments.updated_at_secs = now;
    let version_count = versions.len() as u64;

    if let Err(e) = put_row(host, PAYMENTS, agreement, payments).await {
        return Err(Response::internal_error(e));
    }

    let (message_id, send_state, _) = send_card_and_file(
        host,
        conversation,
        "payment-acknowledgement",
        1,
        &env_json,
        now,
        Some(version_count),
    )
    .await;

    Ok((message_id, send_state))
}
