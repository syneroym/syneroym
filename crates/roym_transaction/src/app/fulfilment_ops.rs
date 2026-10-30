//! Fulfilment receipt signing, retrieval, and verification operations.

use serde::Serialize;
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
    envelope::Response,
    fulfilment::{self, FulfilmentReceiptPayload},
    method::{Reply, VerifyInput},
    record::{Envelope, RECORD_FULFILMENT_RECEIPT},
    transaction::{DealRole, PairState, ReceiptHalf, RecordVerdict, pair_state},
};

use super::{
    AGREEMENTS, AgreementRef, AgreementRow, FULFILMENTS, FenceOutcome, FulfilmentsRow,
    HalfRecordedOutput, LEDGER, PROGRESS, ProgressRow, SendState, booking_ops, get_row,
    ledger::{LedgerKind, LedgerRow, load_booking},
    put_row, resolve_principal_and_owner, send_card_and_file,
};

#[derive(Debug, Serialize)]
pub struct FulfilmentGetOutput {
    pub counterparty: Option<ReceiptHalf>,
    pub quote_author: Option<ReceiptHalf>,
    pub track: TrackState,
}

async fn load_fulfilments_row<H: AppHost>(
    host: &H,
    agreement: &str,
    conversation: &str,
    now: u64,
) -> Result<FulfilmentsRow, Response> {
    match get_row(host, FULFILMENTS, agreement).await {
        Ok(Some(f)) => Ok(f),
        Ok(None) => Ok(FulfilmentsRow {
            agreement: agreement.to_string(),
            conversation: conversation.to_string(),
            counterparty: None,
            quote_author: None,
            updated_at_secs: now,
        }),
        Err(e) => Err(Response::internal_error(e)),
    }
}

pub(crate) async fn sign<H: AppHost>(host: &H, input: AgreementRef) -> Reply<HalfRecordedOutput> {
    let now = clock::now_secs();
    let (principal, owner) = resolve_principal_and_owner(host, now).await?;

    let agr: AgreementRow = match get_row(host, AGREEMENTS, &input.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(Response::invalid_params("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let role = check_fulfilment_preconditions(host, &input.agreement, &agr, &owner).await?;

    let mut fulfilments =
        load_fulfilments_row(host, &input.agreement, &agr.conversation, now).await?;

    let existing = match role {
        DealRole::Counterparty => &fulfilments.counterparty,
        DealRole::QuoteAuthor => &fulfilments.quote_author,
    };
    if let Some(half) = existing {
        let record_id = half.record_id.clone();
        return already_recorded_fulfilment_response(
            host,
            &input.agreement,
            &owner,
            &agr.quote_author_did,
            role,
            &record_id,
            now,
        )
        .await;
    }

    let payload = FulfilmentReceiptPayload {
        agreement: input.agreement.clone(),
        conversation: agr.conversation.clone(),
        counterparty_did: agr.counterparty_did.clone(),
        quote_author_did: agr.quote_author_did.clone(),
        role,
        terms: agr.terms.clone(),
    };
    if let Err(e) = payload.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }

    let (env_json, env, record_id) =
        sign_fulfilment_receipt(host, &input.agreement, &payload, principal).await?;

    let half = ReceiptHalf {
        record_id: record_id.clone(),
        envelope: env_json.clone(),
        issuer: env.issuer,
        issued_at_secs: env.issued_at_secs,
    };

    let fence_claim = FulfilmentFenceClaim {
        agreement: &input.agreement,
        conversation: &agr.conversation,
        role,
        owner: &owner,
        quote_author_did: &agr.quote_author_did,
    };
    match claim_fulfilment_fence(host, fence_claim, &half, now).await? {
        FenceOutcome::Claimed => {}
        FenceOutcome::Answered(out) => return Ok(out),
    }

    match role {
        DealRole::Counterparty => fulfilments.counterparty = Some(half),
        DealRole::QuoteAuthor => fulfilments.quote_author = Some(half),
    }
    fulfilments.updated_at_secs = now;

    if let Err(e) = put_row(host, FULFILMENTS, &input.agreement, &fulfilments).await {
        return Err(Response::internal_error(e));
    }

    let (message_id, send_state, _) =
        send_card_and_file(host, &agr.conversation, "fulfilment-receipt", 1, &env_json, now, None)
            .await;

    maybe_transition_author_fulfilment(host, &input.agreement, &owner, &agr.quote_author_did, now)
        .await;

    Ok(HalfRecordedOutput { record_id, role, message_id: Some(message_id), state: send_state })
}

async fn maybe_transition_author_fulfilment<H: AppHost>(
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
            BookingEvent::Half { track: Track::Fulfilment, role: DealRole::QuoteAuthor },
            now,
        )
        .await;
    }
}

async fn already_recorded_fulfilment_response<H: AppHost>(
    host: &H,
    agreement: &str,
    owner: &str,
    quote_author_did: &str,
    role: DealRole,
    record_id: &str,
    now: u64,
) -> Reply<HalfRecordedOutput> {
    maybe_transition_author_fulfilment(host, agreement, owner, quote_author_did, now).await;
    Ok(HalfRecordedOutput {
        record_id: record_id.to_string(),
        role,
        message_id: None,
        state: SendState::AlreadyRecorded,
    })
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

pub(crate) async fn get<H: AppHost>(host: &H, input: AgreementRef) -> Reply<FulfilmentGetOutput> {
    let _agr: AgreementRow = match get_row(host, AGREEMENTS, &input.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(Response::invalid_params("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let fulfilments: FulfilmentsRow = match get_row(host, FULFILMENTS, &input.agreement).await {
        Ok(Some(f)) => f,
        Ok(None) => FulfilmentsRow::default(),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let track = if let Ok(Some(b)) = load_booking(host, &input.agreement).await {
        b.snapshot.fulfilment
    } else if let Ok(Some(pr)) = get_row::<ProgressRow, _>(host, PROGRESS, &input.agreement).await {
        pr.snapshot.fulfilment
    } else if fulfilments.counterparty.is_some() && fulfilments.quote_author.is_some() {
        TrackState::Acknowledged
    } else if fulfilments.quote_author.is_some() {
        TrackState::Claimed
    } else {
        TrackState::None
    };

    Ok(FulfilmentGetOutput {
        counterparty: fulfilments.counterparty,
        quote_author: fulfilments.quote_author,
        track,
    })
}

async fn check_fulfilment_preconditions<H: AppHost>(
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
    role: DealRole,
    owner: &'a str,
    quote_author_did: &'a str,
}

async fn claim_fulfilment_fence<H: AppHost>(
    host: &H,
    claim: FulfilmentFenceClaim<'_>,
    half: &ReceiptHalf,
    now: u64,
) -> Result<FenceOutcome<HalfRecordedOutput>, Response> {
    let FulfilmentFenceClaim { agreement, conversation, role, owner, quote_author_did } = claim;
    let role_str = role.as_str();
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
        Ok(None) => Ok(FenceOutcome::Claimed),
        Ok(Some(_)) => {
            if let Ok(Some(fence)) = get_row::<LedgerRow, _>(host, LEDGER, &fence_key).await
                && let Some(existing) = &fence.half
            {
                let ctx =
                    FulfilmentRecordCtx { agreement, conversation, role, owner, quote_author_did };
                return ensure_fulfilment_recorded(host, ctx, existing, now)
                    .await
                    .map(FenceOutcome::Answered);
            }
            Err(Response::invalid_params("fulfilment-sign-in-flight"))
        }
        Err(e) => Err(Response::internal_error(e.to_string())),
    }
}

struct FulfilmentRecordCtx<'a> {
    agreement: &'a str,
    conversation: &'a str,
    role: DealRole,
    owner: &'a str,
    quote_author_did: &'a str,
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
) -> Reply<HalfRecordedOutput> {
    let FulfilmentRecordCtx { agreement, conversation, role, owner, quote_author_did } = ctx;
    let mut f: FulfilmentsRow = match get_row(host, FULFILMENTS, agreement).await {
        Ok(Some(row)) => row,
        Ok(None) => FulfilmentsRow {
            agreement: agreement.to_string(),
            conversation: conversation.to_string(),
            counterparty: None,
            quote_author: None,
            updated_at_secs: now,
        },
        Err(e) => return Err(Response::internal_error(e)),
    };
    let slot = match role {
        DealRole::Counterparty => &mut f.counterparty,
        DealRole::QuoteAuthor => &mut f.quote_author,
    };
    let already_present = slot.as_ref().is_some_and(|h| h.record_id == half.record_id);
    if !already_present {
        *slot = Some(half.clone());
        f.updated_at_secs = now;
        if let Err(e) = put_row(host, FULFILMENTS, agreement, &f).await {
            return Err(Response::internal_error(e));
        }
    }

    maybe_transition_author_fulfilment(host, agreement, owner, quote_author_did, now).await;

    if already_present {
        return Ok(HalfRecordedOutput {
            record_id: half.record_id.clone(),
            role,
            message_id: None,
            state: SendState::AlreadyRecorded,
        });
    }
    let (message_id, send_state, _) =
        send_card_and_file(host, conversation, "fulfilment-receipt", 1, &half.envelope, now, None)
            .await;
    Ok(HalfRecordedOutput {
        record_id: half.record_id.clone(),
        role,
        message_id: Some(message_id),
        state: send_state,
    })
}

pub(crate) async fn verify<H: AppHost>(
    _host: &H,
    input: VerifyInput,
) -> Reply<RecordVerdict<FulfilmentReceiptPayload>> {
    syneroym_roym_core::method::verify_with(input, fulfilment::verify_fulfilment_receipt)
}
