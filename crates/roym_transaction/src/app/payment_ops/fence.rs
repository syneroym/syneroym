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

use serde_json::{Value, json};
use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    booking::{BookingEvent, Track},
    envelope::Response,
    transaction::{ReceiptHalf, Role},
};

use super::{PAYMENTS, PaymentsRow, check_acknowledgement_version, maybe_transition_provider_ack};
use crate::app::{
    LEDGER, booking_ops, get_row,
    ledger::{LedgerKind, LedgerRow},
    put_row, send_card_and_file,
};

pub(super) async fn claim_payment_request_fence<H: AppHost>(
    host: &H,
    agreement: &str,
    conversation: &str,
    half: &ReceiptHalf,
    now: u64,
) -> Result<(), Response> {
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
        Ok(None) => Ok(()),
        Ok(Some(_)) => {
            if let Ok(Some(fence)) = get_row::<LedgerRow, _>(host, LEDGER, &fence_key).await
                && let Some(existing) = &fence.half
            {
                return Err(ensure_payment_request_recorded(
                    host,
                    agreement,
                    conversation,
                    existing,
                    now,
                )
                .await);
            }
            if let Ok(Some(py)) = get_row::<PaymentsRow, _>(host, PAYMENTS, agreement).await
                && let Some(existing) = &py.request
            {
                return Err(Response::ok(json!({
                    "record_id": existing.record_id,
                    "message_id": Value::Null,
                    "state": "already-recorded",
                })));
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
) -> Response {
    let mut py: PaymentsRow = match get_row(host, PAYMENTS, agreement).await {
        Ok(Some(p)) => p,
        Ok(None) => PaymentsRow {
            agreement: agreement.to_string(),
            conversation: conversation.to_string(),
            request: None,
            consumer: Vec::new(),
            provider: Vec::new(),
            updated_at_secs: now,
        },
        Err(e) => return Response::internal_error(e),
    };
    if let Some(existing) = &py.request
        && existing.record_id == half.record_id
    {
        return Response::ok(json!({
            "record_id": existing.record_id,
            "message_id": Value::Null,
            "state": "already-recorded",
        }));
    }
    py.request = Some(half.clone());
    py.updated_at_secs = now;
    if let Err(e) = put_row(host, PAYMENTS, agreement, &py).await {
        return Response::internal_error(e);
    }
    let (message_id, send_state, _) =
        send_card_and_file(host, conversation, "payment-request", 1, &half.envelope, now, None)
            .await;
    Response::ok(json!({
        "record_id": half.record_id,
        "message_id": message_id,
        "state": send_state,
    }))
}

pub(super) struct AckFenceClaim<'a> {
    pub(super) agreement: &'a str,
    pub(super) conversation: &'a str,
    pub(super) role: Role,
    pub(super) supersedes: Option<&'a str>,
    pub(super) owner: &'a str,
    pub(super) provider_did: &'a str,
}

pub(super) async fn claim_payment_ack_fence<H: AppHost>(
    host: &H,
    claim: AckFenceClaim<'_>,
    half: &ReceiptHalf,
    now: u64,
) -> Result<(), Response> {
    let AckFenceClaim { agreement, conversation, role, supersedes, owner, provider_did } = claim;
    let ver_tag = supersedes.unwrap_or("first");
    let role_str = match role {
        Role::Consumer => "consumer",
        Role::Provider => "provider",
    };
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
        Ok(None) => Ok(()),
        Ok(Some(_)) => {
            if let Ok(Some(fence)) = get_row::<LedgerRow, _>(host, LEDGER, &ack_fence_key).await
                && let Some(h) = &fence.half
            {
                let ctx = AckRecordCtx { agreement, conversation, role, owner, provider_did };
                return Err(ensure_payment_ack_recorded(host, ctx, h, now).await);
            }
            if let Ok(Some(py)) = get_row::<PaymentsRow, _>(host, PAYMENTS, agreement).await {
                let v = match role {
                    Role::Consumer => &py.consumer,
                    Role::Provider => &py.provider,
                };
                if let Ok(Some(early)) = check_acknowledgement_version(supersedes, v, role) {
                    if owner == provider_did {
                        let _ = booking_ops::transition(
                            host,
                            agreement,
                            BookingEvent::Half { track: Track::Payment, role: Role::Provider },
                            now,
                        )
                        .await;
                    }
                    return Err(early);
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
    role: Role,
    owner: &'a str,
    provider_did: &'a str,
}

async fn ensure_payment_ack_recorded<H: AppHost>(
    host: &H,
    ctx: AckRecordCtx<'_>,
    half: &ReceiptHalf,
    now: u64,
) -> Response {
    let AckRecordCtx { agreement, conversation, role, owner, provider_did } = ctx;
    let mut py: PaymentsRow = match get_row(host, PAYMENTS, agreement).await {
        Ok(Some(p)) => p,
        Ok(None) => PaymentsRow {
            agreement: agreement.to_string(),
            conversation: conversation.to_string(),
            request: None,
            consumer: Vec::new(),
            provider: Vec::new(),
            updated_at_secs: now,
        },
        Err(e) => return Response::internal_error(e),
    };
    let versions = match role {
        Role::Consumer => &mut py.consumer,
        Role::Provider => &mut py.provider,
    };
    let already_present = versions.iter().any(|v| v.record_id == half.record_id);
    if !already_present {
        versions.push(half.clone());
        py.updated_at_secs = now;
        if let Err(e) = put_row(host, PAYMENTS, agreement, &py).await {
            return Response::internal_error(e);
        }
    }

    maybe_transition_provider_ack(host, agreement, owner, provider_did, now).await;

    if already_present {
        return Response::ok(json!({
            "record_id": half.record_id,
            "role": role,
            "message_id": Value::Null,
            "state": "already-recorded",
        }));
    }
    let version_count = match role {
        Role::Consumer => py.consumer.len(),
        Role::Provider => py.provider.len(),
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
    Response::ok(json!({
        "record_id": half.record_id,
        "role": role,
        "message_id": message_id,
        "state": send_state,
    }))
}
