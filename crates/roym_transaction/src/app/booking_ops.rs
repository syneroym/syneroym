//! Booking operations: decision, lifecycle transitions, queries, and history.

use serde::Deserialize;
use serde_json::{Value, json};
use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    booking::{self, BookingEvent, BookingProgressPayload, BookingState, NextStep},
    clock,
    envelope::{Request, Response},
    signing,
    transaction::{PaymentTiming, QuotePayload, Role, pair_state},
};

use super::{
    AGREEMENTS, AgreementRow, BOOKINGS, BookingRow, CARDS, CardRow, LEDGER, PROGRESS, ProgressRow,
    collect_typed, default_list_limit, get_row,
    ledger::{
        Claim, LedgerKind, LedgerRow, StepRow, claim_decision, claim_seat, load_booking, seat_id,
        step_id,
    },
    progress, put_row,
};

pub(crate) async fn decide_booking<H: AppHost>(
    host: &H,
    agreement: &AgreementRow,
    quote: &QuotePayload,
    now: u64,
) -> Result<BookingRow, String> {
    let owner = signing::owner_did(host).await.map_err(|e| e.to_string())?;
    if owner != agreement.provider_did {
        return Err("provider-only".to_string());
    }

    if let Some(row) = load_booking(host, &agreement.quote_record_id).await? {
        return Ok(row);
    }

    let window_end = booking::track_window_end(quote.terms.schedule.as_ref(), now);
    let scheduled = booking::open(
        agreement.quote_record_id.clone(),
        agreement.conversation.clone(),
        agreement.consumer_did.clone(),
        agreement.provider_did.clone(),
        None,
        window_end,
    );
    let scheduled_env = progress::sign(host, &scheduled).await?;

    let outcome = match &quote.slot_id {
        None => {
            claim_decision(
                host,
                &agreement.quote_record_id,
                None,
                None,
                &scheduled,
                &scheduled_env,
                now,
            )
            .await?
        }
        Some(slot) => {
            claim_seat(host, &agreement.quote_record_id, slot, &scheduled, &scheduled_env, now)
                .await?
        }
    };

    match outcome {
        Claim::Won(row) => {
            let row = *row;
            put_row(host, BOOKINGS, &agreement.quote_record_id, &row).await?;
            progress::send(host, &row).await;
            Ok(row)
        }
        Claim::AlreadyDecided => load_booking(host, &agreement.quote_record_id)
            .await?
            .ok_or_else(|| "decision without steps".to_string()),
        Claim::NoSeat(reason) => {
            let conflict = booking::open(
                agreement.quote_record_id.clone(),
                agreement.conversation.clone(),
                agreement.consumer_did.clone(),
                agreement.provider_did.clone(),
                Some(reason),
                window_end,
            );
            let env = progress::sign(host, &conflict).await?;
            match claim_decision(host, &agreement.quote_record_id, None, None, &conflict, &env, now)
                .await?
            {
                Claim::Won(row) => {
                    let row = *row;
                    put_row(host, BOOKINGS, &agreement.quote_record_id, &row).await?;
                    progress::send(host, &row).await;
                    Ok(row)
                }
                _ => load_booking(host, &agreement.quote_record_id)
                    .await?
                    .ok_or_else(|| "decision without steps".to_string()),
            }
        }
    }
}

pub(crate) async fn transition<H: AppHost>(
    host: &H,
    q: &str,
    event: BookingEvent,
    now: u64,
) -> Result<BookingRow, Response> {
    for _attempt in 0..3 {
        let row = match load_booking(host, q).await {
            Ok(Some(r)) => r,
            Ok(None) => return Err(Response::invalid_params("no-booking")),
            Err(e) => return Err(Response::internal_error(e)),
        };

        let next = match booking::apply(&row.snapshot, &event, now) {
            Ok(None) => {
                progress::resend_if_unsent(host, &row).await;
                return Ok(row);
            }
            Ok(Some(mut n)) => {
                n.seq = row.snapshot.seq + 1;
                n
            }
            Err(e) => return Err(Response::invalid_params(e.to_string())),
        };

        let env = match progress::sign(host, &next).await {
            Ok(e) => e,
            Err(e) => return Err(Response::internal_error(e)),
        };

        let step = StepRow {
            seq: next.seq,
            event: event_name(&event),
            snapshot: next.clone(),
            envelope: env.0.clone(),
            record_id: env.1.clone(),
            message_id: None,
        };

        let ledger_row = LedgerRow {
            kind: LedgerKind::Step,
            agreement: q.to_string(),
            slot_id: row.slot_id.clone(),
            seat: row.seat,
            step: Some(step),
            half: None,
            created_at_secs: now,
        };

        let step_write = RecordWriteValue {
            id: step_id(q, next.seq),
            payload: match serde_json::to_vec(&ledger_row) {
                Ok(b) => b,
                Err(e) => return Err(Response::internal_error(e.to_string())),
            },
        };

        match AppDataLayer::create(host, LEDGER.to_string(), vec![step_write]).await {
            Ok(Some(_)) => continue,
            Ok(None) => {}
            Err(e) => return Err(Response::internal_error(e.to_string())),
        }

        if next.state == BookingState::Cancelled
            && let (Some(slot), Some(seat)) = (&row.slot_id, row.seat)
            && let Ok(Some(current_seat)) =
                get_row::<LedgerRow, _>(host, LEDGER, &seat_id(slot, seat)).await
            && current_seat.agreement == row.agreement
        {
            let _ = AppDataLayer::delete(host, LEDGER.to_string(), seat_id(slot, seat)).await;
        }

        let new_row = BookingRow {
            agreement: q.to_string(),
            conversation: next.conversation.clone(),
            state: next.state,
            slot_id: row.slot_id.clone(),
            seat: row.seat,
            snapshot: next,
            progress_record_id: env.1,
            updated_at_secs: now,
        };

        if let Err(e) = put_row(host, BOOKINGS, q, &new_row).await {
            return Err(Response::internal_error(e));
        }

        progress::send(host, &new_row).await;
        return Ok(new_row);
    }
    Err(Response::internal_error("booking is busy; try again"))
}

fn event_name(event: &BookingEvent) -> String {
    match event {
        BookingEvent::Start => "start".to_string(),
        BookingEvent::Cancel { .. } => "cancel".to_string(),
        BookingEvent::Half { .. } => "half".to_string(),
        BookingEvent::Tick => "tick".to_string(),
    }
}

pub(crate) fn booking_view(
    agreement: &str,
    snapshot: Option<&BookingProgressPayload>,
    progress_record_id: Option<&str>,
    writer: Option<&str>,
    agr: &AgreementRow,
    role: Role,
) -> Value {
    let pair = pair_state(agr.consumer.as_ref(), agr.provider.as_ref());
    let next_step_enum = match snapshot {
        Some(s) => booking::next_step(s, agr.terms.payment_timing, role),
        None => match role {
            Role::Provider if agr.terms.payment_timing == PaymentTiming::BeforeWork => {
                NextStep::RequestPayment
            }
            Role::Provider => NextStep::Nothing,
            Role::Consumer => NextStep::WaitForProvider,
        },
    };

    json!({
        "agreement": agreement,
        "writer": writer,
        "seq": snapshot.map(|s| s.seq),
        "state": snapshot.map(|s| s.state),
        "conflict": snapshot.and_then(|s| s.conflict),
        "payment": snapshot.map(|s| s.payment),
        "fulfilment": snapshot.map(|s| s.fulfilment),
        "track_window_ends_at_secs": snapshot.map(|s| s.track_window_ends_at_secs),
        "cancel_reason": snapshot.and_then(|s| s.cancel_reason.as_deref()),
        "progress_record_id": progress_record_id,
        "next": next_step_enum,
        "payment_timing": agr.terms.payment_timing,
        "pair": pair,
    })
}

#[derive(Debug, Deserialize)]
struct AgreementParam {
    agreement: String,
}

pub(crate) async fn booking_get<H: AppHost>(host: &H, req: &Request) -> Response {
    let p: AgreementParam = match serde_json::from_value(req.params.clone()) {
        Ok(v) => v,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    let agr: AgreementRow = match get_row(host, AGREEMENTS, &p.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Response::invalid_params("no-such-agreement"),
        Err(e) => return Response::internal_error(e),
    };

    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let now = clock::now_secs();
    if owner == agr.provider_did {
        if let Ok(Some(row)) = load_booking(host, &p.agreement).await
            && now >= row.snapshot.track_window_ends_at_secs
            && !row.snapshot.state.is_terminal()
        {
            let _ = transition(host, &p.agreement, BookingEvent::Tick, now).await;
        }

        let booking = match load_booking(host, &p.agreement).await {
            Ok(b) => b,
            Err(e) => return Response::internal_error(e),
        };

        let view = match booking {
            Some(b) => booking_view(
                &p.agreement,
                Some(&b.snapshot),
                Some(&b.progress_record_id),
                Some("self"),
                &agr,
                Role::Provider,
            ),
            None => booking_view(&p.agreement, None, None, None, &agr, Role::Provider),
        };
        Response::ok(view)
    } else if owner == agr.consumer_did {
        let progress: Option<ProgressRow> = match get_row(host, PROGRESS, &p.agreement).await {
            Ok(pr) => pr,
            Err(e) => return Response::internal_error(e),
        };

        let view = match progress {
            Some(pr) => booking_view(
                &p.agreement,
                Some(&pr.snapshot),
                Some(&pr.record_id),
                Some("peer"),
                &agr,
                Role::Consumer,
            ),
            None => booking_view(&p.agreement, None, None, None, &agr, Role::Consumer),
        };
        Response::ok(view)
    } else {
        Response::invalid_params("not-a-party")
    }
}

pub(crate) async fn booking_start<H: AppHost>(host: &H, req: &Request) -> Response {
    let p: AgreementParam = match serde_json::from_value(req.params.clone()) {
        Ok(v) => v,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    let agr: AgreementRow = match get_row(host, AGREEMENTS, &p.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Response::invalid_params("no-such-agreement"),
        Err(e) => return Response::internal_error(e),
    };

    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    if owner != agr.provider_did {
        return Response::invalid_params("provider-only");
    }

    let now = clock::now_secs();
    let row = match transition(host, &p.agreement, BookingEvent::Start, now).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    Response::ok(booking_view(
        &p.agreement,
        Some(&row.snapshot),
        Some(&row.progress_record_id),
        Some("self"),
        &agr,
        Role::Provider,
    ))
}

#[derive(Debug, Deserialize)]
struct CancelParams {
    agreement: String,
    reason: String,
}

pub(crate) async fn booking_cancel<H: AppHost>(host: &H, req: &Request) -> Response {
    let p: CancelParams = match serde_json::from_value(req.params.clone()) {
        Ok(v) => v,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    let agr: AgreementRow = match get_row(host, AGREEMENTS, &p.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Response::invalid_params("no-such-agreement"),
        Err(e) => return Response::internal_error(e),
    };

    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    if owner != agr.provider_did {
        return Response::invalid_params("provider-only");
    }

    let now = clock::now_secs();
    let row = match transition(host, &p.agreement, BookingEvent::Cancel { reason: p.reason }, now)
        .await
    {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    Response::ok(booking_view(
        &p.agreement,
        Some(&row.snapshot),
        Some(&row.progress_record_id),
        Some("self"),
        &agr,
        Role::Provider,
    ))
}

pub(crate) async fn booking_history<H: AppHost>(host: &H, req: &Request) -> Response {
    let p: AgreementParam = match serde_json::from_value(req.params.clone()) {
        Ok(v) => v,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    let agr: AgreementRow = match get_row(host, AGREEMENTS, &p.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Response::invalid_params("no-such-agreement"),
        Err(e) => return Response::internal_error(e),
    };

    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    if owner == agr.provider_did {
        let mut steps = Vec::new();
        let mut seq = 1;
        loop {
            let step_key = step_id(&p.agreement, seq);
            let lr: Option<LedgerRow> = match get_row(host, LEDGER, &step_key).await {
                Ok(r) => r,
                Err(e) => return Response::internal_error(e),
            };
            match lr.and_then(|r| r.step) {
                Some(step) => {
                    steps.push(json!({
                        "seq": step.seq,
                        "event": step.event,
                        "snapshot": step.snapshot,
                        "envelope": step.envelope,
                        "record_id": step.record_id,
                    }));
                    seq += 1;
                }
                None => break,
            }
        }
        Response::ok(json!({ "history": steps }))
    } else if owner == agr.consumer_did {
        let filter = json!({ "conversation": agr.conversation }).to_string();
        let cards: Vec<CardRow> = match collect_typed(host, CARDS, Some(filter)).await {
            Ok(c) => c,
            Err(resp) => return resp,
        };
        let mut history = Vec::new();
        for card in cards {
            if card.card_type == "booking-progress"
                && card.verified
                && let Some(data) = &card.data
                && data.get("agreement").and_then(Value::as_str) == Some(&p.agreement)
            {
                history.push(json!({
                    "seq": data.get("seq"),
                    "snapshot": data,
                    "record_id": card.record_id,
                }));
            }
        }
        Response::ok(json!({ "history": history }))
    } else {
        Response::invalid_params("not-a-party")
    }
}

#[derive(Debug, Deserialize)]
struct BookingListParams {
    #[serde(default)]
    conversation: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default = "default_list_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}

pub(crate) async fn booking_list<H: AppHost>(host: &H, req: &Request) -> Response {
    let p: BookingListParams = match serde_json::from_value(req.params.clone()) {
        Ok(v) => v,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let mut views = Vec::new();
    let filter = p.conversation.as_ref().map(|c| json!({ "conversation": c }).to_string());

    let provider_bookings: Vec<BookingRow> =
        match collect_typed(host, BOOKINGS, filter.clone()).await {
            Ok(b) => b,
            Err(resp) => return resp,
        };

    for b in provider_bookings {
        if let Some(ref st) = p.state
            && serde_json::to_value(b.state).ok().and_then(|v| v.as_str().map(ToString::to_string))
                != Some(st.clone())
        {
            continue;
        }
        if let Ok(Some(agr)) = get_row::<AgreementRow, _>(host, AGREEMENTS, &b.agreement).await
            && (owner == agr.provider_did || owner == agr.consumer_did)
        {
            let role = if owner == agr.provider_did { Role::Provider } else { Role::Consumer };
            views.push(booking_view(
                &b.agreement,
                Some(&b.snapshot),
                Some(&b.progress_record_id),
                Some("self"),
                &agr,
                role,
            ));
        }
    }

    if views.is_empty() {
        let consumer_progress: Vec<ProgressRow> = match collect_typed(host, PROGRESS, filter).await
        {
            Ok(pr) => pr,
            Err(resp) => return resp,
        };
        for pr in consumer_progress {
            if let Some(ref st) = p.state
                && serde_json::to_value(pr.snapshot.state)
                    .ok()
                    .and_then(|v| v.as_str().map(ToString::to_string))
                    != Some(st.clone())
            {
                continue;
            }
            if let Ok(Some(agr)) = get_row::<AgreementRow, _>(host, AGREEMENTS, &pr.agreement).await
                && (owner == agr.provider_did || owner == agr.consumer_did)
            {
                let role = if owner == agr.provider_did { Role::Provider } else { Role::Consumer };
                views.push(booking_view(
                    &pr.agreement,
                    Some(&pr.snapshot),
                    Some(&pr.record_id),
                    Some("peer"),
                    &agr,
                    role,
                ));
            }
        }
    }

    let paged: Vec<Value> = views.into_iter().skip(p.offset).take(p.limit).collect();
    Response::ok(json!({ "bookings": paged }))
}
