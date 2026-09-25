//! Payment request, acknowledgement, retrieval, and verification operations.

mod fence;

use fence::{AckFenceClaim, claim_payment_ack_fence, claim_payment_request_fence};
use serde::Deserialize;
use serde_json::{Value, json};
use syneroym_app_host::{
    AppHost, AppSigning,
    types::signing::{Principal, RecordDraft},
};
use syneroym_roym_core::{
    booking::{BookingEvent, BookingState, Track, TrackState},
    clock,
    envelope::{Request, Response},
    payment::{self, PaymentAcknowledgementPayload, PaymentRequestPayload},
    record::{Envelope, RECORD_PAYMENT_ACKNOWLEDGEMENT, RECORD_PAYMENT_REQUEST},
    transaction::{PairState, ReceiptHalf, Role, pair_state},
    verdict::RecordVerdict,
};

use super::{
    AGREEMENTS, AgreementRow, BookingRow, PAYMENTS, PROGRESS, PaymentsRow, ProgressRow,
    booking_ops, get_row, ledger::load_booking, put_row, resolve_principal_and_owner,
    send_card_and_file,
};

#[derive(Debug, Deserialize)]
struct PaymentRequestParams {
    agreement: String,
    #[serde(default)]
    note: Option<String>,
}

pub(crate) async fn payment_request<H: AppHost>(host: &H, req: &Request) -> Response {
    let p: PaymentRequestParams = match serde_json::from_value(req.params.clone()) {
        Ok(v) => v,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    let now = clock::now_secs();
    let (principal, owner) = match resolve_principal_and_owner(host, now).await {
        Ok(res) => res,
        Err(resp) => return resp,
    };

    let agr: AgreementRow = match get_row(host, AGREEMENTS, &p.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Response::invalid_params("no-such-agreement"),
        Err(e) => return Response::internal_error(e),
    };

    let booking = match load_booking(host, &p.agreement).await {
        Ok(Some(b)) => b,
        Ok(None) => return Response::invalid_params("no-booking"),
        Err(e) => return Response::internal_error(e),
    };

    if let Err(resp) = validate_payment_request_preconditions(&agr, &booking, &owner) {
        return resp;
    }

    let mut payments: PaymentsRow = match get_row(host, PAYMENTS, &p.agreement).await {
        Ok(Some(py)) => py,
        Ok(None) => PaymentsRow {
            agreement: p.agreement.clone(),
            conversation: agr.conversation.clone(),
            request: None,
            consumer: Vec::new(),
            provider: Vec::new(),
            updated_at_secs: now,
        },
        Err(e) => return Response::internal_error(e),
    };

    if let Some(existing) = &payments.request {
        return Response::ok(json!({
            "record_id": existing.record_id,
            "message_id": Value::Null,
            "state": "already-recorded",
        }));
    }

    let payload = PaymentRequestPayload {
        agreement: p.agreement.clone(),
        conversation: agr.conversation.clone(),
        consumer_did: agr.consumer_did.clone(),
        provider_did: agr.provider_did.clone(),
        currency: agr.terms.currency.clone(),
        amount_minor: agr.terms.amount_minor,
        note: p.note,
    };
    if let Err(e) = payload.validate() {
        return Response::invalid_params(e.to_string());
    }

    let (env_json, env, record_id) =
        match sign_payment_request(host, &p.agreement, &payload, principal).await {
            Ok(triplet) => triplet,
            Err(resp) => return resp,
        };

    let half = ReceiptHalf {
        record_id: record_id.clone(),
        envelope: env_json.clone(),
        issuer: env.issuer,
        issued_at_secs: env.issued_at_secs,
    };

    if let Err(resp) =
        claim_payment_request_fence(host, &p.agreement, &agr.conversation, &half, now).await
    {
        return resp;
    }

    payments.request = Some(half);
    payments.updated_at_secs = now;
    if let Err(e) = put_row(host, PAYMENTS, &p.agreement, &payments).await {
        return Response::internal_error(e);
    }

    let (message_id, send_state, _) =
        send_card_and_file(host, &agr.conversation, "payment-request", 1, &env_json, now, None)
            .await;

    Response::ok(json!({
        "record_id": record_id,
        "message_id": message_id,
        "state": send_state,
    }))
}

#[derive(Debug, Deserialize)]
struct AcknowledgeParams {
    agreement: String,
    #[serde(default)]
    observed_at_secs: Option<u64>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    reference: Option<String>,
    #[serde(default)]
    supersedes: Option<String>,
}

struct AckContext {
    principal: Principal,
    owner: String,
    agr: AgreementRow,
    role: Role,
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
            consumer: Vec::new(),
            provider: Vec::new(),
            updated_at_secs: now,
        },
        Err(e) => return Err(Response::internal_error(e)),
    };
    Ok(AckContext { principal, owner, agr, role, payments })
}

async fn maybe_transition_provider_ack<H: AppHost>(
    host: &H,
    agreement: &str,
    owner: &str,
    provider_did: &str,
    now: u64,
) {
    if owner == provider_did {
        let _ = booking_ops::transition(
            host,
            agreement,
            BookingEvent::Half { track: Track::Payment, role: Role::Provider },
            now,
        )
        .await;
    }
}

pub(crate) async fn payment_acknowledge<H: AppHost>(host: &H, req: &Request) -> Response {
    let p: AcknowledgeParams = match serde_json::from_value(req.params.clone()) {
        Ok(v) => v,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    let now = clock::now_secs();
    let AckContext { principal, owner, agr, role, mut payments } =
        match load_ack_context(host, &p.agreement, now).await {
            Ok(ctx) => ctx,
            Err(resp) => return resp,
        };

    let versions = match role {
        Role::Consumer => &mut payments.consumer,
        Role::Provider => &mut payments.provider,
    };

    if let Some(early_resp) =
        match check_acknowledgement_version(p.supersedes.as_deref(), versions, role) {
            Ok(opt) => opt,
            Err(resp) => return resp,
        }
    {
        maybe_transition_provider_ack(host, &p.agreement, &owner, &agr.provider_did, now).await;
        return early_resp;
    }

    let payload = match build_payment_ack_payload(&p, &agr, role, now) {
        Ok(pl) => pl,
        Err(resp) => return resp,
    };

    if let Err(resp) =
        check_acknowledgement_correction(p.supersedes.as_deref(), versions, &payload, now)
    {
        return resp;
    }

    let (env_json, env, record_id) =
        match sign_payment_ack(host, &p.agreement, &payload, p.supersedes.clone(), principal).await
        {
            Ok(triplet) => triplet,
            Err(resp) => return resp,
        };

    let half = ReceiptHalf {
        record_id: record_id.clone(),
        envelope: env_json,
        issuer: env.issuer,
        issued_at_secs: env.issued_at_secs,
    };

    let ack_fence_claim = AckFenceClaim {
        agreement: &p.agreement,
        conversation: &agr.conversation,
        role,
        supersedes: p.supersedes.as_deref(),
        owner: &owner,
        provider_did: &agr.provider_did,
    };
    if let Err(resp) = claim_payment_ack_fence(host, ack_fence_claim, &half, now).await {
        return resp;
    }

    let (message_id, send_state) = match record_and_send_ack(
        host,
        &p.agreement,
        &agr.conversation,
        &mut payments,
        role,
        half,
        now,
    )
    .await
    {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };

    maybe_transition_provider_ack(host, &p.agreement, &owner, &agr.provider_did, now).await;

    Response::ok(json!({
        "record_id": record_id,
        "role": role,
        "message_id": message_id,
        "state": send_state,
    }))
}

async fn check_booking_not_terminal<H: AppHost>(
    host: &H,
    agreement: &str,
    role: Role,
) -> Option<Response> {
    match role {
        Role::Provider => {
            if let Ok(Some(b)) = load_booking(host, agreement).await {
                if b.state == BookingState::Conflict {
                    return Some(Response::invalid_params("booking-conflict"));
                }
                if b.state == BookingState::Cancelled {
                    return Some(Response::invalid_params("booking-cancelled"));
                }
            }
        }
        Role::Consumer => {
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
    let pair = pair_state(agr.consumer.as_ref(), agr.provider.as_ref());
    if pair != PairState::Complete {
        return Err(Response::invalid_params("agreement-incomplete"));
    }
    if owner != agr.provider_did {
        return Err(Response::invalid_params("provider-only"));
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
) -> Result<Role, Response> {
    let pair = pair_state(agr.consumer.as_ref(), agr.provider.as_ref());
    if pair != PairState::Complete {
        return Err(Response::invalid_params("agreement-incomplete"));
    }

    let role = if owner == agr.provider_did {
        Role::Provider
    } else if owner == agr.consumer_did {
        Role::Consumer
    } else {
        return Err(Response::invalid_params("not-a-party"));
    };

    if let Some(err) = check_booking_not_terminal(host, agreement, role).await {
        return Err(err);
    }
    Ok(role)
}

fn check_acknowledgement_version(
    supersedes: Option<&str>,
    versions: &[ReceiptHalf],
    role: Role,
) -> Result<Option<Response>, Response> {
    match supersedes {
        None if !versions.is_empty() => {
            let rec_id = versions.last().map(|v| v.record_id.as_str()).unwrap_or_default();
            Ok(Some(Response::ok(json!({
                "record_id": rec_id,
                "role": role,
                "message_id": Value::Null,
                "state": "already-recorded",
            }))))
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

#[derive(Debug, Deserialize)]
struct AgreementParam {
    agreement: String,
}

pub(crate) async fn payment_get<H: AppHost>(host: &H, req: &Request) -> Response {
    let p: AgreementParam = match serde_json::from_value(req.params.clone()) {
        Ok(v) => v,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    let _agr: AgreementRow = match get_row(host, AGREEMENTS, &p.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Response::invalid_params("no-such-agreement"),
        Err(e) => return Response::internal_error(e),
    };

    let payments: PaymentsRow = match get_row(host, PAYMENTS, &p.agreement).await {
        Ok(Some(py)) => py,
        Ok(None) => PaymentsRow::default(),
        Err(e) => return Response::internal_error(e),
    };

    let track = if let Ok(Some(b)) = load_booking(host, &p.agreement).await {
        b.snapshot.payment
    } else if let Ok(Some(pr)) = get_row::<ProgressRow, _>(host, PROGRESS, &p.agreement).await {
        pr.snapshot.payment
    } else if !payments.provider.is_empty() {
        TrackState::Acknowledged
    } else if !payments.consumer.is_empty() {
        TrackState::Claimed
    } else {
        TrackState::None
    };

    Response::ok(json!({
        "request": payments.request,
        "consumer": payments.consumer,
        "provider": payments.provider,
        "track": track,
    }))
}

pub(crate) async fn payment_verify<H: AppHost>(_host: &H, req: &Request) -> Response {
    let now = clock::now_secs();
    let env_val = match req.params.get("envelope") {
        Some(v) => v,
        None => return Response::invalid_params("envelope is required"),
    };
    let env_str = match env_val {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let envelope = match Envelope::from_json(&env_str) {
        Ok(e) => e,
        Err(e) => {
            return Response::ok(json!(RecordVerdict::<()>::refused(format!(
                "envelope is not valid json: {e}"
            ))));
        }
    };
    match envelope.record_type.as_str() {
        RECORD_PAYMENT_REQUEST => {
            Response::ok(json!(payment::verify_payment_request(&env_str, now)))
        }
        RECORD_PAYMENT_ACKNOWLEDGEMENT => {
            Response::ok(json!(payment::verify_payment_acknowledgement(&env_str, now)))
        }
        _ => Response::ok(json!(RecordVerdict::<()>::refused("not a payment record type"))),
    }
}

fn build_payment_ack_payload(
    p: &AcknowledgeParams,
    agr: &AgreementRow,
    role: Role,
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
        consumer_did: agr.consumer_did.clone(),
        provider_did: agr.provider_did.clone(),
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
    role: Role,
    half: ReceiptHalf,
    now: u64,
) -> Result<(String, String), Response> {
    let env_json = half.envelope.clone();
    let versions = match role {
        Role::Consumer => &mut payments.consumer,
        Role::Provider => &mut payments.provider,
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
