//! Single-writer fencing for `payment.request`/`payment.acknowledge`, and
//! crash-recovery self-healing.

use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    deal::SettlementRole,
    envelope::Response,
    method::Reply,
    progress::AgreementEvent,
    transaction::{DealRole, ReceiptHalf},
};

use super::{
    PAYMENTS, PaymentRequestOutput, PaymentsRow, check_acknowledgement_version, payment_id,
};
use crate::app::{
    FenceOutcome, HalfRecordedOutput, LEDGER, OutgoingCard, SendState, disclosures, get_row,
    ledger::{LedgerKind, LedgerRow},
    progress_ops, put_row, send_card_and_file,
};

pub(super) async fn claim_payment_request_fence<H: AppHost>(
    host: &H,
    agreement: &str,
    commitment_id: &str,
    conversation: &str,
    half: &ReceiptHalf,
    now: u64,
) -> Result<FenceOutcome<PaymentRequestOutput>, Response> {
    let fence_key = format!("payreq:{agreement}:{commitment_id}");
    let fence_row = LedgerRow {
        kind: LedgerKind::Fence,
        agreement: agreement.to_string(),
        state: None,
        quote_id: None,
        commitment_id: Some(commitment_id.to_string()),
        reason: None,
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
            handle_payment_request_fence_collision(
                host,
                agreement,
                commitment_id,
                conversation,
                &fence_key,
                now,
            )
            .await
        }
        Err(e) => Err(Response::internal_error(e.to_string())),
    }
}

async fn handle_payment_request_fence_collision<H: AppHost>(
    host: &H,
    agreement: &str,
    commitment_id: &str,
    conversation: &str,
    fence_key: &str,
    now: u64,
) -> Result<FenceOutcome<PaymentRequestOutput>, Response> {
    if let Ok(Some(fence)) = get_row::<LedgerRow, _>(host, LEDGER, fence_key).await
        && let Some(existing) = &fence.half
    {
        return ensure_payment_request_recorded(
            host,
            agreement,
            commitment_id,
            conversation,
            existing,
            now,
        )
        .await
        .map(FenceOutcome::Answered);
    }
    let p_id = payment_id(agreement, commitment_id);
    if let Ok(Some(py)) = get_row::<PaymentsRow, _>(host, PAYMENTS, &p_id).await
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

pub(crate) async fn ensure_payment_request_recorded<H: AppHost>(
    host: &H,
    agreement: &str,
    commitment_id: &str,
    conversation: &str,
    half: &ReceiptHalf,
    now: u64,
) -> Reply<PaymentRequestOutput> {
    let p_id = payment_id(agreement, commitment_id);
    let mut py: PaymentsRow = match get_row(host, PAYMENTS, &p_id).await {
        Ok(Some(p)) => p,
        Ok(None) => PaymentsRow {
            agreement: agreement.to_string(),
            commitment_id: commitment_id.to_string(),
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
    if let Err(e) = put_row(host, PAYMENTS, &p_id, &py).await {
        return Err(Response::internal_error(e));
    }
    let stored_disc = disclosures::load(host, &half.record_id)
        .await
        .map_err(Response::internal_error)?
        .ok_or_else(|| {
            Response::internal_error(format!("disclosures missing for {}", half.record_id))
        })?;
    let card = OutgoingCard {
        card_type: "payment-request",
        version: 1,
        envelope: &half.envelope,
        send: &stored_disc,
        view: &stored_disc,
    };
    let (message_id, send_state, _) =
        send_card_and_file(host, conversation, &card, now, None).await;
    Ok(PaymentRequestOutput {
        record_id: half.record_id.clone(),
        message_id: Some(message_id),
        state: send_state,
    })
}

pub(super) struct AckFenceClaim<'a> {
    pub(super) agreement: &'a str,
    pub(super) commitment_id: &'a str,
    pub(super) conversation: &'a str,
    pub(super) role: DealRole,
    pub(super) settlement_role: SettlementRole,
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
    let AckFenceClaim {
        agreement,
        commitment_id,
        conversation,
        role,
        settlement_role,
        supersedes,
        owner,
        quote_author_did,
    } = claim;
    let ver_tag = supersedes.unwrap_or("first");
    let role_str = role.as_str();
    let ack_fence_key = format!("ack:{agreement}:{commitment_id}:{role_str}:{ver_tag}");
    let ack_fence_row = LedgerRow {
        kind: LedgerKind::Fence,
        agreement: agreement.to_string(),
        state: None,
        quote_id: None,
        commitment_id: Some(commitment_id.to_string()),
        reason: None,
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
            let ctx = AckRecordCtx {
                agreement,
                commitment_id,
                conversation,
                role,
                settlement_role,
                owner,
                quote_author_did,
            };
            handle_payment_ack_fence_collision(host, ctx, &ack_fence_key, supersedes, now).await
        }
        Err(e) => Err(Response::internal_error(e.to_string())),
    }
}

pub(crate) struct AckRecordCtx<'a> {
    pub(crate) agreement: &'a str,
    pub(crate) commitment_id: &'a str,
    pub(crate) conversation: &'a str,
    pub(crate) role: DealRole,
    pub(crate) settlement_role: SettlementRole,
    pub(crate) owner: &'a str,
    pub(crate) quote_author_did: &'a str,
}

async fn handle_payment_ack_fence_collision<H: AppHost>(
    host: &H,
    ctx: AckRecordCtx<'_>,
    ack_fence_key: &str,
    supersedes: Option<&str>,
    now: u64,
) -> Result<FenceOutcome<HalfRecordedOutput>, Response> {
    if let Ok(Some(fence)) = get_row::<LedgerRow, _>(host, LEDGER, ack_fence_key).await
        && let Some(h) = &fence.half
    {
        return ensure_payment_ack_recorded(host, ctx, h, now).await.map(FenceOutcome::Answered);
    }
    let p_id = payment_id(ctx.agreement, ctx.commitment_id);
    if let Ok(Some(py)) = get_row::<PaymentsRow, _>(host, PAYMENTS, &p_id).await {
        let v = match ctx.role {
            DealRole::Counterparty => &py.counterparty,
            DealRole::QuoteAuthor => &py.quote_author,
        };
        if let Ok(Some(early)) = check_acknowledgement_version(supersedes, v, ctx.role) {
            if ctx.owner == ctx.quote_author_did {
                let _ = progress_ops::transition_at(
                    host,
                    ctx.agreement,
                    AgreementEvent::Half {
                        commitment_id: ctx.commitment_id.to_string(),
                        role: ctx.settlement_role,
                        record_id: early.record_id.clone(),
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

pub(crate) async fn ensure_payment_ack_recorded<H: AppHost>(
    host: &H,
    ctx: AckRecordCtx<'_>,
    half: &ReceiptHalf,
    now: u64,
) -> Reply<HalfRecordedOutput> {
    let AckRecordCtx {
        agreement,
        commitment_id,
        conversation,
        role,
        settlement_role,
        owner,
        quote_author_did,
    } = ctx;
    let p_id = payment_id(agreement, commitment_id);
    let mut py: PaymentsRow = match get_row(host, PAYMENTS, &p_id).await {
        Ok(Some(p)) => p,
        Ok(None) => PaymentsRow {
            agreement: agreement.to_string(),
            commitment_id: commitment_id.to_string(),
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
        if let Err(e) = put_row(host, PAYMENTS, &p_id, &py).await {
            return Err(Response::internal_error(e));
        }
    }

    if owner == quote_author_did {
        let _ = progress_ops::transition_at(
            host,
            agreement,
            AgreementEvent::Half {
                commitment_id: commitment_id.to_string(),
                role: settlement_role,
                record_id: half.record_id.clone(),
            },
            now,
        )
        .await;
    }

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
    let stored_disc = disclosures::load(host, &half.record_id)
        .await
        .map_err(Response::internal_error)?
        .ok_or_else(|| {
            Response::internal_error(format!("disclosures missing for {}", half.record_id))
        })?;
    let card = OutgoingCard {
        card_type: "payment-acknowledgement",
        version: 1,
        envelope: &half.envelope,
        send: &stored_disc,
        view: &stored_disc,
    };
    let (message_id, send_state, _) =
        send_card_and_file(host, conversation, &card, now, Some(version_count)).await;
    Ok(HalfRecordedOutput {
        record_id: half.record_id.clone(),
        role,
        message_id: Some(message_id),
        state: send_state,
    })
}
