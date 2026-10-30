//! Single-writer fencing for `payment.request`/`payment.acknowledge`, and
//! the crash-recovery self-heal that goes with it: the fence row and the
//! shared `PAYMENTS` row are written in two separate calls, so a crash
//! between them would otherwise strand a signed half in the fence with
//! nothing in the shared row and no card ever sent. Both fence-claim
//! functions route their "already claimed" branch through the matching
//! `ensure_*_recorded` helper, which writes the shared row (and sends the
//! card) if it does not already hold this half -- a normal concurrent
//! retry that loses the race heals the row the same way a crash-recovery
//! retry would.

use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    booking::{BookingEvent, Track},
    envelope::Response,
    method::Reply,
    transaction::{DealRole, ReceiptHalf},
};

use super::{
    PAYMENTS, PaymentRequestOutput, PaymentsRow, check_acknowledgement_version,
    maybe_transition_author_ack,
};
use crate::app::{
    FenceOutcome, HalfRecordedOutput, LEDGER, SendState, booking_ops, get_row,
    ledger::{LedgerKind, LedgerRow},
    put_row, send_card_and_file,
};

pub(super) async fn claim_payment_request_fence<H: AppHost>(
    host: &H,
    agreement: &str,
    conversation: &str,
    half: &ReceiptHalf,
    now: u64,
) -> Result<FenceOutcome<PaymentRequestOutput>, Response> {
    let fence_key = format!("payreq:{agreement}");
    let fence_row = LedgerRow {
        kind: LedgerKind::Fence,
        agreement: agreement.to_string(),
        slot_id: None,
        seat: None,
        step: None,
        half: Some(half.clone()),
        created_at_secs: now,
    };
    let fence_write = match serde_json::to_vec(&fence_row) {
        Ok(b) => RecordWriteValue { id: fence_key.clone(), payload: b },
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    match AppDataLayer::create(host, LEDGER.to_string(), vec![fence_write]).await {
        Ok(None) => Ok(FenceOutcome::Claimed),
        Ok(Some(_)) => {
            if let Ok(Some(fence)) = get_row::<LedgerRow, _>(host, LEDGER, &fence_key).await
                && let Some(existing) = &fence.half
            {
                return ensure_payment_request_recorded(
                    host,
                    agreement,
                    conversation,
                    existing,
                    now,
                )
                .await
                .map(FenceOutcome::Answered);
            }
            if let Ok(Some(py)) = get_row::<PaymentsRow, _>(host, PAYMENTS, agreement).await
                && let Some(existing) = &py.request
            {
                return Ok(FenceOutcome::Answered(PaymentRequestOutput {
                    record_id: existing.record_id.clone(),
                    message_id: None,
                    state: SendState::AlreadyRecorded,
                }));
            }
            Err(Response::invalid_params("payment-request-in-flight"))
        }
        Err(e) => Err(Response::internal_error(e.to_string())),
    }
}

async fn ensure_payment_request_recorded<H: AppHost>(
    host: &H,
    agreement: &str,
    conversation: &str,
    half: &ReceiptHalf,
    now: u64,
) -> Reply<PaymentRequestOutput> {
    let mut py: PaymentsRow = match get_row(host, PAYMENTS, agreement).await {
        Ok(Some(p)) => p,
        Ok(None) => PaymentsRow {
            agreement: agreement.to_string(),
            conversation: conversation.to_string(),
            request: None,
            counterparty: Vec::new(),
            quote_author: Vec::new(),
            updated_at_secs: now,
        },
        Err(e) => return Err(Response::internal_error(e)),
    };
    if let Some(existing) = &py.request
        && existing.record_id == half.record_id
    {
        return Ok(PaymentRequestOutput {
            record_id: existing.record_id.clone(),
            message_id: None,
            state: SendState::AlreadyRecorded,
        });
    }
    py.request = Some(half.clone());
    py.updated_at_secs = now;
    if let Err(e) = put_row(host, PAYMENTS, agreement, &py).await {
        return Err(Response::internal_error(e));
    }
    let (message_id, send_state, _) =
        send_card_and_file(host, conversation, "payment-request", 1, &half.envelope, now, None)
            .await;
    Ok(PaymentRequestOutput {
        record_id: half.record_id.clone(),
        message_id: Some(message_id),
        state: send_state,
    })
}

pub(super) struct AckFenceClaim<'a> {
    pub(super) agreement: &'a str,
    pub(super) conversation: &'a str,
    pub(super) role: DealRole,
    pub(super) supersedes: Option<&'a str>,
    pub(super) owner: &'a str,
    pub(super) quote_author_did: &'a str,
}

pub(super) async fn claim_payment_ack_fence<H: AppHost>(
    host: &H,
    claim: AckFenceClaim<'_>,
    half: &ReceiptHalf,
    now: u64,
) -> Result<FenceOutcome<HalfRecordedOutput>, Response> {
    let AckFenceClaim { agreement, conversation, role, supersedes, owner, quote_author_did } =
        claim;
    let ver_tag = supersedes.unwrap_or("first");
    let role_str = role.as_str();
    let ack_fence_key = format!("ack:{agreement}:{role_str}:{ver_tag}");
    let ack_fence_row = LedgerRow {
        kind: LedgerKind::Fence,
        agreement: agreement.to_string(),
        slot_id: None,
        seat: None,
        step: None,
        half: Some(half.clone()),
        created_at_secs: now,
    };
    let ack_fence_write = match serde_json::to_vec(&ack_fence_row) {
        Ok(b) => RecordWriteValue { id: ack_fence_key.clone(), payload: b },
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    match AppDataLayer::create(host, LEDGER.to_string(), vec![ack_fence_write]).await {
        Ok(None) => Ok(FenceOutcome::Claimed),
        Ok(Some(_)) => {
            if let Ok(Some(fence)) = get_row::<LedgerRow, _>(host, LEDGER, &ack_fence_key).await
                && let Some(h) = &fence.half
            {
                let ctx = AckRecordCtx { agreement, conversation, role, owner, quote_author_did };
                return ensure_payment_ack_recorded(host, ctx, h, now)
                    .await
                    .map(FenceOutcome::Answered);
            }
            if let Ok(Some(py)) = get_row::<PaymentsRow, _>(host, PAYMENTS, agreement).await {
                let v = match role {
                    DealRole::Counterparty => &py.counterparty,
                    DealRole::QuoteAuthor => &py.quote_author,
                };
                if let Ok(Some(early)) = check_acknowledgement_version(supersedes, v, role) {
                    if owner == quote_author_did {
                        let _ = booking_ops::transition(
                            host,
                            agreement,
                            BookingEvent::Half {
                                track: Track::Payment,
                                role: DealRole::QuoteAuthor,
                            },
                            now,
                        )
                        .await;
                    }
                    return Ok(FenceOutcome::Answered(early));
                }
            }
            Err(Response::invalid_params("payment-acknowledgement-in-flight"))
        }
        Err(e) => Err(Response::internal_error(e.to_string())),
    }
}

struct AckRecordCtx<'a> {
    agreement: &'a str,
    conversation: &'a str,
    role: DealRole,
    owner: &'a str,
    quote_author_did: &'a str,
}

async fn ensure_payment_ack_recorded<H: AppHost>(
    host: &H,
    ctx: AckRecordCtx<'_>,
    half: &ReceiptHalf,
    now: u64,
) -> Reply<HalfRecordedOutput> {
    let AckRecordCtx { agreement, conversation, role, owner, quote_author_did } = ctx;
    let mut py: PaymentsRow = match get_row(host, PAYMENTS, agreement).await {
        Ok(Some(p)) => p,
        Ok(None) => PaymentsRow {
            agreement: agreement.to_string(),
            conversation: conversation.to_string(),
            request: None,
            counterparty: Vec::new(),
            quote_author: Vec::new(),
            updated_at_secs: now,
        },
        Err(e) => return Err(Response::internal_error(e)),
    };
    let versions = match role {
        DealRole::Counterparty => &mut py.counterparty,
        DealRole::QuoteAuthor => &mut py.quote_author,
    };
    let already_present = versions.iter().any(|v| v.record_id == half.record_id);
    if !already_present {
        versions.push(half.clone());
        py.updated_at_secs = now;
        if let Err(e) = put_row(host, PAYMENTS, agreement, &py).await {
            return Err(Response::internal_error(e));
        }
    }

    maybe_transition_author_ack(host, agreement, owner, quote_author_did, now).await;

    if already_present {
        return Ok(HalfRecordedOutput {
            record_id: half.record_id.clone(),
            role,
            message_id: None,
            state: SendState::AlreadyRecorded,
        });
    }
    let version_count = match role {
        DealRole::Counterparty => py.counterparty.len(),
        DealRole::QuoteAuthor => py.quote_author.len(),
    } as u64;
    let (message_id, send_state, _) = send_card_and_file(
        host,
        conversation,
        "payment-acknowledgement",
        1,
        &half.envelope,
        now,
        Some(version_count),
    )
    .await;
    Ok(HalfRecordedOutput {
        record_id: half.record_id.clone(),
        role,
        message_id: Some(message_id),
        state: send_state,
    })
}
