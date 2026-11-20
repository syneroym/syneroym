//! Payment request, acknowledgement, retrieval, and verification operations.

mod fence;

use fence::{AckFenceClaim, claim_payment_ack_fence, claim_payment_request_fence};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_app_host::{
    AppHost, AppSigning,
    types::signing::{Principal, RecordDraft},
};
use syneroym_roym_core::{
    clock,
    deal::{CommitmentKind, LocalText, SettlementRole, transition::CommitmentState},
    disclosure::{PAYMENT_ACK_SET, PAYMENT_REQUEST_SET, conceal_hidden},
    envelope::Response,
    method::{self, DisclosedVerifyInput, Reply},
    payment::{
        self, PaymentAckHidden, PaymentAcknowledgementPayload, PaymentRequestHidden,
        PaymentRequestPayload,
    },
    progress::AgreementEvent,
    record::{Envelope, RECORD_PAYMENT_ACKNOWLEDGEMENT, RECORD_PAYMENT_REQUEST},
    transaction::{DealRole, PairState, ReceiptHalf, pair_state},
    verdict::RecordVerdict,
};

use super::{
    AGREEMENTS, AgreementRow, FenceOutcome, HalfRecordedOutput, OutgoingCard, PAYMENTS,
    PaymentsRow, SendState, disclosures, get_row, progress_ops, put_row,
    resolve_principal_and_owner, send_card_and_file,
};

pub(crate) fn payment_id(agreement: &str, commitment_id: &str) -> String {
    format!("{agreement}:{commitment_id}")
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentRequestInput {
    pub agreement: String,
    pub commitment_id: String,
    #[serde(default)]
    pub note: Option<LocalText>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentRequestOutput {
    pub record_id: String,
    pub message_id: Option<String>,
    pub state: SendState,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentAckInput {
    pub agreement: String,
    pub commitment_id: String,
    #[serde(default)]
    pub observed_at_secs: Option<u64>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub settlement_note: Option<LocalText>,
    #[serde(default)]
    pub supersedes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentKeyInput {
    pub agreement: String,
    pub commitment_id: String,
}

#[derive(Debug, Serialize)]
pub struct PaymentGetOutput {
    pub request: Option<ReceiptHalf>,
    pub counterparty: Vec<ReceiptHalf>,
    pub quote_author: Vec<ReceiptHalf>,
    pub track: CommitmentState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentVerdict {
    pub kind: String,
    pub verdict: RecordVerdict<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown: Option<Value>,
    pub hidden: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disclosure_refused: Option<String>,
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

    validate_payment_request_preconditions(&agr, &input.commitment_id, &owner)?;

    let p_id = payment_id(&input.agreement, &input.commitment_id);
    let mut payments: PaymentsRow = match get_row(host, PAYMENTS, &p_id).await {
        Ok(Some(py)) => py,
        Ok(None) => PaymentsRow {
            agreement: input.agreement.clone(),
            commitment_id: input.commitment_id.clone(),
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

    let hidden = PaymentRequestHidden { note: input.note };
    if let Err(e) = hidden.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }
    let mut salts = disclosures::fresh_salts(host, 1).await?.into_iter();
    let (sd, disc) = conceal_hidden(&hidden, &PAYMENT_REQUEST_SET, &mut salts)
        .map_err(|e| Response::invalid_params(e.to_string()))?;

    let payload = PaymentRequestPayload {
        agreement: input.agreement.clone(),
        conversation: agr.conversation.clone(),
        commitment_id: input.commitment_id.clone(),
        quote_author_did: agr.quote_author_did.clone(),
        counterparty_did: agr.counterparty_did.clone(),
        sd,
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

    disclosures::store(host, &record_id, RECORD_PAYMENT_REQUEST, &disc, now)
        .await
        .map_err(Response::internal_error)?;

    match claim_payment_request_fence(
        host,
        &input.agreement,
        &input.commitment_id,
        &agr.conversation,
        &half,
        now,
    )
    .await?
    {
        FenceOutcome::Claimed => {}
        FenceOutcome::Answered(out) => return Ok(out),
    }

    payments.request = Some(half);
    payments.updated_at_secs = now;
    if let Err(e) = put_row(host, PAYMENTS, &p_id, &payments).await {
        return Err(Response::internal_error(e));
    }

    if owner == agr.quote_author_did {
        let _ = progress_ops::transition_at(
            host,
            &input.agreement,
            AgreementEvent::PaymentRequest { commitment_id: input.commitment_id.clone() },
            now,
        )
        .await;
    }

    let card = OutgoingCard {
        card_type: "payment-request",
        version: 1,
        envelope: &env_json,
        send: &disc,
        view: &disc,
    };

    let (message_id, send_state, _) =
        send_card_and_file(host, &agr.conversation, &card, now, None).await;

    Ok(PaymentRequestOutput { record_id, message_id: Some(message_id), state: send_state })
}

async fn load_or_init_payments_row<H: AppHost>(
    host: &H,
    p_id: &str,
    agreement: &str,
    commitment_id: &str,
    conversation: &str,
    now: u64,
) -> Result<PaymentsRow, Response> {
    match get_row(host, PAYMENTS, p_id).await {
        Ok(Some(py)) => Ok(py),
        Ok(None) => Ok(PaymentsRow {
            agreement: agreement.to_string(),
            commitment_id: commitment_id.to_string(),
            conversation: conversation.to_string(),
            request: None,
            counterparty: Vec::new(),
            quote_author: Vec::new(),
            updated_at_secs: now,
        }),
        Err(e) => Err(Response::internal_error(e)),
    }
}

async fn prepare_and_sign_ack<H: AppHost>(
    host: &H,
    agreement: &str,
    payload: &PaymentAcknowledgementPayload,
    supersedes: Option<String>,
    principal: Principal,
) -> Result<ReceiptHalf, Response> {
    if let Err(e) = payload.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }
    let (env_json, env, record_id) =
        sign_payment_ack(host, agreement, payload, supersedes, principal).await?;
    Ok(ReceiptHalf {
        record_id,
        envelope: env_json,
        issuer: env.issuer,
        issued_at_secs: env.issued_at_secs,
    })
}

pub(crate) async fn acknowledge<H: AppHost>(
    host: &H,
    input: PaymentAckInput,
) -> Reply<HalfRecordedOutput> {
    let now = clock::now_secs();
    let (principal, owner) = resolve_principal_and_owner(host, now).await?;

    let agr: AgreementRow = match get_row(host, AGREEMENTS, &input.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(Response::invalid_params("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let (deal_role, settlement_role) =
        check_acknowledgement_preconditions(&agr, &input.commitment_id, &owner, &input.method)?;

    let p_id = payment_id(&input.agreement, &input.commitment_id);
    let mut payments = load_or_init_payments_row(
        host,
        &p_id,
        &input.agreement,
        &input.commitment_id,
        &agr.conversation,
        now,
    )
    .await?;

    let versions = match deal_role {
        DealRole::Counterparty => &payments.counterparty,
        DealRole::QuoteAuthor => &payments.quote_author,
    };
    if let Some(early) =
        check_acknowledgement_version(input.supersedes.as_deref(), versions, deal_role)?
    {
        return Ok(early);
    }

    let (payload, disc) = prepare_ack_payload(host, &input, &agr, settlement_role, now).await?;

    check_acknowledgement_correction(input.supersedes.as_deref(), versions, &payload, now)?;

    let half =
        prepare_and_sign_ack(host, &input.agreement, &payload, input.supersedes.clone(), principal)
            .await?;

    disclosures::store(host, &half.record_id, RECORD_PAYMENT_ACKNOWLEDGEMENT, &disc, now)
        .await
        .map_err(Response::internal_error)?;

    let ack_fence_claim = AckFenceClaim {
        agreement: &input.agreement,
        commitment_id: &input.commitment_id,
        conversation: &agr.conversation,
        role: deal_role,
        settlement_role,
        supersedes: input.supersedes.as_deref(),
        owner: &owner,
        quote_author_did: &agr.quote_author_did,
    };
    match claim_payment_ack_fence(host, ack_fence_claim, &half, now).await? {
        FenceOutcome::Claimed => {}
        FenceOutcome::Answered(out) => return Ok(out),
    }

    let send_ctx = AckSendContext { conversation: &agr.conversation, disclosures: &disc, now };
    let (message_id, send_state) =
        record_and_send_ack(host, &p_id, &mut payments, deal_role, half.clone(), send_ctx).await?;

    if owner == agr.quote_author_did {
        let _ = progress_ops::transition_at(
            host,
            &input.agreement,
            AgreementEvent::Half {
                commitment_id: input.commitment_id.clone(),
                role: settlement_role,
                record_id: half.record_id.clone(),
            },
            now,
        )
        .await;
    }

    Ok(HalfRecordedOutput {
        record_id: half.record_id,
        role: deal_role,
        message_id: Some(message_id),
        state: send_state,
    })
}

pub(crate) async fn get<H: AppHost>(host: &H, input: PaymentKeyInput) -> Reply<PaymentGetOutput> {
    let agr: AgreementRow = match get_row(host, AGREEMENTS, &input.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(Response::invalid_params("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let commitment =
        match agr.terms.commitments.iter().find(|c| c.commitment_id == input.commitment_id) {
            Some(c) => c,
            None => return Err(Response::invalid_params("no-such-commitment")),
        };
    if commitment.kind != CommitmentKind::Currency {
        return Err(Response::invalid_params("not-currency-commitment"));
    }

    let p_id = payment_id(&input.agreement, &input.commitment_id);
    let payments: PaymentsRow = match get_row(host, PAYMENTS, &p_id).await {
        Ok(Some(py)) => py,
        Ok(None) => PaymentsRow {
            agreement: input.agreement.clone(),
            commitment_id: input.commitment_id.clone(),
            conversation: agr.conversation.clone(),
            request: None,
            counterparty: Vec::new(),
            quote_author: Vec::new(),
            updated_at_secs: agr.updated_at_secs,
        },
        Err(e) => return Err(Response::internal_error(e)),
    };

    let track = agr
        .commitments
        .iter()
        .find(|c| c.commitment_id == input.commitment_id)
        .map(|c| c.state)
        .unwrap_or(CommitmentState::Promised);

    Ok(PaymentGetOutput {
        request: payments.request,
        counterparty: payments.counterparty,
        quote_author: payments.quote_author,
        track,
    })
}

pub(crate) async fn verify<H: AppHost>(
    _host: &H,
    input: DisclosedVerifyInput,
) -> Reply<PaymentVerdict> {
    let now = clock::now_secs();
    let v_req = payment::verify_payment_request(&input.envelope, now);
    if v_req.verified {
        let (shown, hidden, disclosure_refused) = if let Some(ref p) = v_req.payload {
            method::reveal_disclosed_payment_request(p, &input.disclosures)
        } else {
            (None, method::all_payment_request_hidden(), None)
        };
        return Ok(PaymentVerdict {
            kind: "payment-request".to_string(),
            verdict: RecordVerdict {
                verified: true,
                expired: false,
                reason: None,
                revocation_status: v_req.revocation_status,
                record_id: v_req.record_id,
                issuer: v_req.issuer,
                signer_did: v_req.signer_did,
                issued_at_secs: v_req.issued_at_secs,
                expires_at_secs: None,
                supersedes: v_req.supersedes,
                payload: v_req.payload.map(|p| serde_json::to_value(p).unwrap_or_default()),
            },
            shown,
            hidden,
            disclosure_refused,
        });
    }

    let v_ack = payment::verify_payment_acknowledgement(&input.envelope, now);
    if v_ack.verified {
        let (shown, hidden, disclosure_refused) = if let Some(ref p) = v_ack.payload {
            method::reveal_disclosed_payment_ack(p, &input.disclosures)
        } else {
            (None, method::all_payment_ack_hidden(), None)
        };
        return Ok(PaymentVerdict {
            kind: "payment-acknowledgement".to_string(),
            verdict: RecordVerdict {
                verified: true,
                expired: false,
                reason: None,
                revocation_status: v_ack.revocation_status,
                record_id: v_ack.record_id,
                issuer: v_ack.issuer,
                signer_did: v_ack.signer_did,
                issued_at_secs: v_ack.issued_at_secs,
                expires_at_secs: None,
                supersedes: v_ack.supersedes,
                payload: v_ack.payload.map(|p| serde_json::to_value(p).unwrap_or_default()),
            },
            shown,
            hidden,
            disclosure_refused,
        });
    }

    Ok(PaymentVerdict {
        kind: "unknown".to_string(),
        verdict: RecordVerdict::refused(
            "envelope did not verify as payment request or acknowledgement",
        ),
        shown: None,
        hidden: Vec::new(),
        disclosure_refused: None,
    })
}

fn validate_payment_request_preconditions(
    agr: &AgreementRow,
    commitment_id: &str,
    owner: &str,
) -> Result<(), Response> {
    let pair = pair_state(agr.counterparty.as_ref(), agr.quote_author.as_ref());
    if pair != PairState::Complete {
        return Err(Response::invalid_params("agreement-incomplete"));
    }
    if let Some(state) = agr.state
        && state.is_terminal()
    {
        return Err(Response::invalid_params("agreement-terminal"));
    }

    let c = agr
        .terms
        .commitments
        .iter()
        .find(|c| c.commitment_id == commitment_id)
        .ok_or_else(|| Response::invalid_params("no-such-commitment"))?;
    if c.kind != CommitmentKind::Currency {
        return Err(Response::invalid_params("not-currency-commitment"));
    }

    let receiver_did = if c.giver == DealRole::QuoteAuthor {
        &agr.counterparty_did
    } else {
        &agr.quote_author_did
    };
    if owner != receiver_did {
        return Err(Response::invalid_params("receiver-only"));
    }

    Ok(())
}

fn check_acknowledgement_preconditions(
    agr: &AgreementRow,
    commitment_id: &str,
    owner: &str,
    method: &Option<String>,
) -> Result<(DealRole, SettlementRole), Response> {
    let pair = pair_state(agr.counterparty.as_ref(), agr.quote_author.as_ref());
    if pair != PairState::Complete {
        return Err(Response::invalid_params("agreement-incomplete"));
    }
    if let Some(state) = agr.state
        && state.is_terminal()
    {
        return Err(Response::invalid_params("agreement-terminal"));
    }

    let c = agr
        .terms
        .commitments
        .iter()
        .find(|c| c.commitment_id == commitment_id)
        .ok_or_else(|| Response::invalid_params("no-such-commitment"))?;
    if c.kind != CommitmentKind::Currency {
        return Err(Response::invalid_params("not-currency-commitment"));
    }

    if let Some(m) = method
        && !payment::commitment_accepts(c, m.as_str())
    {
        return Err(Response::invalid_params("method-not-in-terms"));
    }

    let deal_role = if owner == agr.quote_author_did {
        DealRole::QuoteAuthor
    } else if owner == agr.counterparty_did {
        DealRole::Counterparty
    } else {
        return Err(Response::invalid_params("not-a-party"));
    };

    let settlement_role =
        if deal_role == c.giver { SettlementRole::Giver } else { SettlementRole::Receiver };

    Ok((deal_role, settlement_role))
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

async fn prepare_ack_payload<H: AppHost>(
    host: &H,
    input: &PaymentAckInput,
    agr: &AgreementRow,
    settlement_role: SettlementRole,
    now: u64,
) -> Result<(PaymentAcknowledgementPayload, Vec<String>), Response> {
    let hidden = PaymentAckHidden {
        method: input.method.clone(),
        reference: input.reference.clone(),
        settlement_note: input.settlement_note.clone(),
    };
    if let Err(e) = hidden.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }
    let mut salts = disclosures::fresh_salts(host, 3).await?.into_iter();
    let (sd, disc) = conceal_hidden(&hidden, &PAYMENT_ACK_SET, &mut salts)
        .map_err(|e| Response::invalid_params(e.to_string()))?;

    let payload = PaymentAcknowledgementPayload {
        agreement: input.agreement.clone(),
        conversation: agr.conversation.clone(),
        commitment_id: input.commitment_id.clone(),
        quote_author_did: agr.quote_author_did.clone(),
        counterparty_did: agr.counterparty_did.clone(),
        role: settlement_role,
        observed_at_secs: input.observed_at_secs.unwrap_or(now),
        sd,
    };
    Ok((payload, disc))
}

struct AckSendContext<'a> {
    conversation: &'a str,
    disclosures: &'a [String],
    now: u64,
}

async fn record_and_send_ack<H: AppHost>(
    host: &H,
    p_id: &str,
    payments: &mut PaymentsRow,
    role: DealRole,
    half: ReceiptHalf,
    ctx: AckSendContext<'_>,
) -> Result<(String, SendState), Response> {
    let versions = match role {
        DealRole::Counterparty => &mut payments.counterparty,
        DealRole::QuoteAuthor => &mut payments.quote_author,
    };
    versions.push(half.clone());
    payments.updated_at_secs = ctx.now;
    if let Err(e) = put_row(host, PAYMENTS, p_id, payments).await {
        return Err(Response::internal_error(e));
    }

    let version_count = match role {
        DealRole::Counterparty => payments.counterparty.len(),
        DealRole::QuoteAuthor => payments.quote_author.len(),
    } as u64;

    let card = OutgoingCard {
        card_type: "payment-acknowledgement",
        version: 1,
        envelope: &half.envelope,
        send: ctx.disclosures,
        view: ctx.disclosures,
    };

    let (message_id, send_state, _) =
        send_card_and_file(host, ctx.conversation, &card, ctx.now, Some(version_count)).await;

    Ok((message_id, send_state))
}
