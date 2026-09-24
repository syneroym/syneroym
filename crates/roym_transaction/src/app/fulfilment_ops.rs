//! Fulfilment receipt signing, retrieval, and verification operations.

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
    fulfilment::{self, FulfilmentReceiptPayload},
    record::{Envelope, RECORD_FULFILMENT_RECEIPT},
    transaction::{PairState, ReceiptHalf, Role, pair_state},
};

use super::{
    AGREEMENTS, AgreementRow, FULFILMENTS, FulfilmentsRow, PROGRESS, ProgressRow, booking_ops,
    get_row, ledger::load_booking, put_row, resolve_principal_and_owner, send_card_and_file,
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
        return Response::ok(json!({
            "record_id": half.record_id,
            "role": role,
            "message_id": Value::Null,
            "state": "already-recorded",
        }));
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

    if owner == agr.provider_did {
        let _ = booking_ops::transition(
            host,
            &p.agreement,
            BookingEvent::Half { track: Track::Fulfilment, role: Role::Provider },
            now,
        )
        .await;
    }

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
