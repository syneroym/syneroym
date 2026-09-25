//! Fulfilment receipt signing, retrieval, and verification operations.

use serde::Deserialize;
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost, AppSigning,
    types::{
        data_layer::RecordWriteValue,
        signing::{Principal, RecordDraft},
    },
};
use syneroym_roym_core::{
    booking::{BookingEvent, BookingState, Track, TrackState},
    clock,
    envelope::{Request, Response},
    fulfilment::{self, FulfilmentReceiptPayload},
    record::{Envelope, RECORD_FULFILMENT_RECEIPT},
    transaction::{PairState, ReceiptHalf, Role, pair_state},
};

use super::{
    AGREEMENTS, AgreementRow, FULFILMENTS, FulfilmentsRow, LEDGER, PROGRESS, ProgressRow,
    booking_ops, get_row,
    ledger::{LedgerKind, LedgerRow, load_booking},
    put_row, resolve_principal_and_owner, send_card_and_file,
};

#[derive(Debug, Deserialize)]
struct AgreementParam {
    agreement: String,
}

pub(crate) async fn fulfilment_sign<H: AppHost>(host: &H, req: &Request) -> Response {
    let p: AgreementParam = match serde_json::from_value(req.params.clone()) {
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

    let role = match check_fulfilment_preconditions(host, &p.agreement, &agr, &owner).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let mut fulfilments: FulfilmentsRow = match get_row(host, FULFILMENTS, &p.agreement).await {
        Ok(Some(f)) => f,
        Ok(None) => FulfilmentsRow {
            agreement: p.agreement.clone(),
            conversation: agr.conversation.clone(),
            consumer: None,
            provider: None,
            updated_at_secs: now,
        },
        Err(e) => return Response::internal_error(e),
    };

    let existing = match role {
        Role::Consumer => &fulfilments.consumer,
        Role::Provider => &fulfilments.provider,
    };
    if let Some(half) = existing {
        let record_id = half.record_id.clone();
        return already_recorded_fulfilment_response(
            host,
            &p.agreement,
            &owner,
            &agr.provider_did,
            role,
            &record_id,
            now,
        )
        .await;
    }

    let payload = FulfilmentReceiptPayload {
        agreement: p.agreement.clone(),
        conversation: agr.conversation.clone(),
        consumer_did: agr.consumer_did.clone(),
        provider_did: agr.provider_did.clone(),
        role,
        terms: agr.terms.clone(),
    };
    if let Err(e) = payload.validate() {
        return Response::invalid_params(e.to_string());
    }

    let (env_json, env, record_id) =
        match sign_fulfilment_receipt(host, &p.agreement, &payload, principal).await {
            Ok(triplet) => triplet,
            Err(resp) => return resp,
        };

    let half = ReceiptHalf {
        record_id: record_id.clone(),
        envelope: env_json.clone(),
        issuer: env.issuer,
        issued_at_secs: env.issued_at_secs,
    };

    let fence_claim = FulfilmentFenceClaim {
        agreement: &p.agreement,
        conversation: &agr.conversation,
        role,
        owner: &owner,
        provider_did: &agr.provider_did,
    };
    if let Err(resp) = claim_fulfilment_fence(host, fence_claim, &half, now).await {
        return resp;
    }

    match role {
        Role::Consumer => fulfilments.consumer = Some(half),
        Role::Provider => fulfilments.provider = Some(half),
    }
    fulfilments.updated_at_secs = now;

    if let Err(e) = put_row(host, FULFILMENTS, &p.agreement, &fulfilments).await {
        return Response::internal_error(e);
    }

    let (message_id, send_state, _) =
        send_card_and_file(host, &agr.conversation, "fulfilment-receipt", 1, &env_json, now, None)
            .await;

    maybe_transition_provider_fulfilment(host, &p.agreement, &owner, &agr.provider_did, now).await;

    Response::ok(json!({
        "record_id": record_id,
        "role": role,
        "message_id": message_id,
        "state": send_state,
    }))
}

async fn maybe_transition_provider_fulfilment<H: AppHost>(
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
            BookingEvent::Half { track: Track::Fulfilment, role: Role::Provider },
            now,
        )
        .await;
    }
}

async fn already_recorded_fulfilment_response<H: AppHost>(
    host: &H,
    agreement: &str,
    owner: &str,
    provider_did: &str,
    role: Role,
    record_id: &str,
    now: u64,
) -> Response {
    maybe_transition_provider_fulfilment(host, agreement, owner, provider_did, now).await;
    Response::ok(json!({
        "record_id": record_id,
        "role": role,
        "message_id": Value::Null,
        "state": "already-recorded",
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

pub(crate) async fn fulfilment_get<H: AppHost>(host: &H, req: &Request) -> Response {
    let p: AgreementParam = match serde_json::from_value(req.params.clone()) {
        Ok(v) => v,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    let _agr: AgreementRow = match get_row(host, AGREEMENTS, &p.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Response::invalid_params("no-such-agreement"),
        Err(e) => return Response::internal_error(e),
    };

    let fulfilments: FulfilmentsRow = match get_row(host, FULFILMENTS, &p.agreement).await {
        Ok(Some(f)) => f,
        Ok(None) => FulfilmentsRow::default(),
        Err(e) => return Response::internal_error(e),
    };

    let track = if let Ok(Some(b)) = load_booking(host, &p.agreement).await {
        b.snapshot.fulfilment
    } else if let Ok(Some(pr)) = get_row::<ProgressRow, _>(host, PROGRESS, &p.agreement).await {
        pr.snapshot.fulfilment
    } else if fulfilments.consumer.is_some() && fulfilments.provider.is_some() {
        TrackState::Acknowledged
    } else if fulfilments.provider.is_some() {
        TrackState::Claimed
    } else {
        TrackState::None
    };

    Response::ok(json!({
        "consumer": fulfilments.consumer,
        "provider": fulfilments.provider,
        "track": track,
    }))
}

async fn check_fulfilment_preconditions<H: AppHost>(
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

async fn sign_fulfilment_receipt<H: AppHost>(
    host: &H,
    agreement: &str,
    payload: &FulfilmentReceiptPayload,
    principal: Principal,
) -> Result<(String, Envelope, String), Response> {
    let payload_str = match serde_json::to_string(payload) {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let draft = RecordDraft {
        version: fulfilment::FULFILMENT_RECEIPT_VERSION,
        record_type: RECORD_FULFILMENT_RECEIPT.to_string(),
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

struct FulfilmentFenceClaim<'a> {
    agreement: &'a str,
    conversation: &'a str,
    role: Role,
    owner: &'a str,
    provider_did: &'a str,
}

async fn claim_fulfilment_fence<H: AppHost>(
    host: &H,
    claim: FulfilmentFenceClaim<'_>,
    half: &ReceiptHalf,
    now: u64,
) -> Result<(), Response> {
    let FulfilmentFenceClaim { agreement, conversation, role, owner, provider_did } = claim;
    let role_str = match role {
        Role::Consumer => "consumer",
        Role::Provider => "provider",
    };
    let fence_key = format!("fulfil:{agreement}:{role_str}");
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
                let ctx =
                    FulfilmentRecordCtx { agreement, conversation, role, owner, provider_did };
                return Err(ensure_fulfilment_recorded(host, ctx, existing, now).await);
            }
            Err(Response::invalid_params("fulfilment-sign-in-flight"))
        }
        Err(e) => Err(Response::internal_error(e.to_string())),
    }
}

struct FulfilmentRecordCtx<'a> {
    agreement: &'a str,
    conversation: &'a str,
    role: Role,
    owner: &'a str,
    provider_did: &'a str,
}

/// Mirrors `payment_ops::ensure_payment_ack_recorded`: a crash between
/// claiming the fulfilment fence and writing the shared `FULFILMENTS` row
/// would otherwise strand a signed half that no retry ever files or
/// sends -- every later call would just answer `already-recorded` against
/// a row nothing ever wrote. Called only from the fence's "already
/// claimed" branch.
async fn ensure_fulfilment_recorded<H: AppHost>(
    host: &H,
    ctx: FulfilmentRecordCtx<'_>,
    half: &ReceiptHalf,
    now: u64,
) -> Response {
    let FulfilmentRecordCtx { agreement, conversation, role, owner, provider_did } = ctx;
    let mut f: FulfilmentsRow = match get_row(host, FULFILMENTS, agreement).await {
        Ok(Some(row)) => row,
        Ok(None) => FulfilmentsRow {
            agreement: agreement.to_string(),
            conversation: conversation.to_string(),
            consumer: None,
            provider: None,
            updated_at_secs: now,
        },
        Err(e) => return Response::internal_error(e),
    };
    let slot = match role {
        Role::Consumer => &mut f.consumer,
        Role::Provider => &mut f.provider,
    };
    let already_present = slot.as_ref().is_some_and(|h| h.record_id == half.record_id);
    if !already_present {
        *slot = Some(half.clone());
        f.updated_at_secs = now;
        if let Err(e) = put_row(host, FULFILMENTS, agreement, &f).await {
            return Response::internal_error(e);
        }
    }

    maybe_transition_provider_fulfilment(host, agreement, owner, provider_did, now).await;

    if already_present {
        return Response::ok(json!({
            "record_id": half.record_id,
            "role": role,
            "message_id": Value::Null,
            "state": "already-recorded",
        }));
    }
    let (message_id, send_state, _) =
        send_card_and_file(host, conversation, "fulfilment-receipt", 1, &half.envelope, now, None)
            .await;
    Response::ok(json!({
        "record_id": half.record_id,
        "role": role,
        "message_id": message_id,
        "state": send_state,
    }))
}
