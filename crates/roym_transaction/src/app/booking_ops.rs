//! Booking operations: decision, lifecycle transitions, queries, and history.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    booking::{
        self, BookingEvent, BookingProgressPayload, BookingState, ConflictReason, NextStep,
        TrackState,
    },
    clock,
    envelope::Response,
    method::{self, Reply, VerifyInput},
    paging, signing,
    transaction::{DealRole, PairState, PaymentTiming, QuotePayload, RecordVerdict, pair_state},
};

use super::{
    AGREEMENTS, AgreementRef, AgreementRow, BOOKINGS, BookingRow, BookingWriter, CARDS, CardRow,
    LEDGER, PROGRESS, ProgressRow, get_row,
    ledger::{
        Claim, LedgerKind, LedgerRow, StepRow, claim_decision, claim_seat, load_booking, seat_id,
        step_id,
    },
    progress, put_row,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookingCancelInput {
    pub agreement: String,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookingListInput {
    #[serde(default)]
    pub conversation: Option<String>,
    #[serde(default)]
    pub state: Option<BookingState>,
    #[serde(default = "super::default_list_limit_u64")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

#[derive(Debug, Serialize)]
pub struct BookingListOutput {
    pub bookings: Vec<BookingView>,
}

#[derive(Debug, Serialize)]
pub struct BookingHistoryOutput {
    pub history: Vec<BookingHistoryStep>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum BookingHistoryStep {
    Author(AuthorStep),
    Peer(PeerStep),
}

#[derive(Debug, Serialize)]
pub struct AuthorStep {
    pub seq: u32,
    pub event: String,
    pub snapshot: BookingProgressPayload,
    pub envelope: String,
    pub record_id: String,
}

#[derive(Debug, Serialize)]
pub struct PeerStep {
    pub seq: Option<u32>,
    pub snapshot: Value,
    pub record_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookingView {
    pub agreement: String,
    pub writer: Option<BookingWriter>,
    pub seq: Option<u32>,
    pub state: Option<BookingState>,
    pub conflict: Option<ConflictReason>,
    pub payment: Option<TrackState>,
    pub fulfilment: Option<TrackState>,
    pub track_window_ends_at_secs: Option<u64>,
    pub cancel_reason: Option<String>,
    pub progress_record_id: Option<String>,
    pub next: NextStep,
    pub payment_timing: PaymentTiming,
    pub pair: PairState,
}

pub(crate) async fn decide_booking<H: AppHost>(
    host: &H,
    agreement: &AgreementRow,
    quote: &QuotePayload,
    now: u64,
) -> Result<BookingRow, String> {
    let owner = signing::owner_did(host).await.map_err(|e| e.to_string())?;
    if owner != agreement.quote_author_did {
        return Err("quote-author-only".to_string());
    }

    if let Some(row) = load_booking(host, &agreement.quote_record_id).await? {
        return Ok(row);
    }

    let window_end = booking::track_window_end(quote.terms.schedule.as_ref(), now);
    let scheduled = booking::open(
        agreement.quote_record_id.clone(),
        agreement.conversation.clone(),
        agreement.counterparty_did.clone(),
        agreement.quote_author_did.clone(),
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
                agreement.counterparty_did.clone(),
                agreement.quote_author_did.clone(),
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
    writer: Option<BookingWriter>,
    agr: &AgreementRow,
    role: DealRole,
) -> BookingView {
    let pair = pair_state(agr.counterparty.as_ref(), agr.quote_author.as_ref());
    let next_step_enum = match snapshot {
        Some(s) => booking::next_step(s, agr.terms.payment_timing, role),
        None => match role {
            DealRole::QuoteAuthor if agr.terms.payment_timing == PaymentTiming::BeforeWork => {
                NextStep::RequestPayment
            }
            DealRole::QuoteAuthor => NextStep::Nothing,
            DealRole::Counterparty => NextStep::WaitForQuoteAuthor,
        },
    };

    BookingView {
        agreement: agreement.to_string(),
        writer,
        seq: snapshot.map(|s| s.seq),
        state: snapshot.map(|s| s.state),
        conflict: snapshot.and_then(|s| s.conflict),
        payment: snapshot.map(|s| s.payment),
        fulfilment: snapshot.map(|s| s.fulfilment),
        track_window_ends_at_secs: snapshot.map(|s| s.track_window_ends_at_secs),
        cancel_reason: snapshot.and_then(|s| s.cancel_reason.clone()),
        progress_record_id: progress_record_id.map(str::to_string),
        next: next_step_enum,
        payment_timing: agr.terms.payment_timing,
        pair,
    }
}

pub(crate) async fn verify<H: AppHost>(
    _host: &H,
    input: VerifyInput,
) -> Reply<RecordVerdict<BookingProgressPayload>> {
    method::verify_with(input, booking::verify_booking_progress)
}

pub(crate) async fn booking_get<H: AppHost>(host: &H, input: AgreementRef) -> Reply<BookingView> {
    let agr: AgreementRow = match get_row(host, AGREEMENTS, &input.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(Response::invalid_params("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let now = clock::now_secs();
    if owner == agr.quote_author_did {
        if let Ok(Some(row)) = load_booking(host, &input.agreement).await
            && now >= row.snapshot.track_window_ends_at_secs
            && !row.snapshot.state.is_terminal()
        {
            let _ = transition(host, &input.agreement, BookingEvent::Tick, now).await;
        }

        let booking = match load_booking(host, &input.agreement).await {
            Ok(b) => b,
            Err(e) => return Err(Response::internal_error(e)),
        };

        let view = match booking {
            Some(b) => booking_view(
                &input.agreement,
                Some(&b.snapshot),
                Some(&b.progress_record_id),
                Some(BookingWriter::Mine),
                &agr,
                DealRole::QuoteAuthor,
            ),
            None => booking_view(&input.agreement, None, None, None, &agr, DealRole::QuoteAuthor),
        };
        Ok(view)
    } else if owner == agr.counterparty_did {
        let progress: Option<ProgressRow> = match get_row(host, PROGRESS, &input.agreement).await {
            Ok(pr) => pr,
            Err(e) => return Err(Response::internal_error(e)),
        };

        let view = match progress {
            Some(pr) => booking_view(
                &input.agreement,
                Some(&pr.snapshot),
                Some(&pr.record_id),
                Some(BookingWriter::Peer),
                &agr,
                DealRole::Counterparty,
            ),
            None => booking_view(&input.agreement, None, None, None, &agr, DealRole::Counterparty),
        };
        Ok(view)
    } else {
        Err(Response::invalid_params("not-a-party"))
    }
}

pub(crate) async fn booking_start<H: AppHost>(host: &H, input: AgreementRef) -> Reply<BookingView> {
    let agr: AgreementRow = match get_row(host, AGREEMENTS, &input.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(Response::invalid_params("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    if owner != agr.quote_author_did {
        return Err(Response::invalid_params("quote-author-only"));
    }

    let now = clock::now_secs();
    let row = transition(host, &input.agreement, BookingEvent::Start, now).await?;

    Ok(booking_view(
        &input.agreement,
        Some(&row.snapshot),
        Some(&row.progress_record_id),
        Some(BookingWriter::Mine),
        &agr,
        DealRole::QuoteAuthor,
    ))
}

pub(crate) async fn booking_cancel<H: AppHost>(
    host: &H,
    input: BookingCancelInput,
) -> Reply<BookingView> {
    let agr: AgreementRow = match get_row(host, AGREEMENTS, &input.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(Response::invalid_params("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    if owner != agr.quote_author_did {
        return Err(Response::invalid_params("quote-author-only"));
    }

    let now = clock::now_secs();
    let row =
        transition(host, &input.agreement, BookingEvent::Cancel { reason: input.reason }, now)
            .await?;

    Ok(booking_view(
        &input.agreement,
        Some(&row.snapshot),
        Some(&row.progress_record_id),
        Some(BookingWriter::Mine),
        &agr,
        DealRole::QuoteAuthor,
    ))
}

pub(crate) async fn history<H: AppHost>(
    host: &H,
    input: AgreementRef,
) -> Reply<BookingHistoryOutput> {
    let agr: AgreementRow = match get_row(host, AGREEMENTS, &input.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(Response::invalid_params("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    if owner == agr.quote_author_did {
        let mut steps = Vec::new();
        let mut seq = 1;
        loop {
            let step_key = step_id(&input.agreement, seq);
            let lr: Option<LedgerRow> = match get_row(host, LEDGER, &step_key).await {
                Ok(r) => r,
                Err(e) => return Err(Response::internal_error(e)),
            };
            match lr.and_then(|r| r.step) {
                Some(step) => {
                    steps.push(BookingHistoryStep::Author(AuthorStep {
                        seq: step.seq,
                        event: step.event,
                        snapshot: step.snapshot,
                        envelope: step.envelope,
                        record_id: step.record_id,
                    }));
                    seq += 1;
                }
                None => break,
            }
        }
        Ok(BookingHistoryOutput { history: steps })
    } else if owner == agr.counterparty_did {
        let filter = json!({ "conversation": agr.conversation }).to_string();
        let cards: Vec<CardRow> = match paging::query_all(host, CARDS, Some(filter)).await {
            Ok(c) => c,
            Err(e) => return Err(Response::internal_error(e)),
        };
        let mut history = Vec::new();
        for card in cards {
            if card.card_type == "booking-progress"
                && card.verified
                && let Some(data) = &card.data
                && data.get("agreement").and_then(Value::as_str) == Some(&input.agreement)
            {
                let seq =
                    data.get("seq").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok());
                history.push(BookingHistoryStep::Peer(PeerStep {
                    seq,
                    snapshot: data.clone(),
                    record_id: card.record_id,
                }));
            }
        }
        Ok(BookingHistoryOutput { history })
    } else {
        Err(Response::invalid_params("not-a-party"))
    }
}

pub(crate) async fn booking_list<H: AppHost>(
    host: &H,
    input: BookingListInput,
) -> Reply<BookingListOutput> {
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let mut views = Vec::new();
    let filter = input.conversation.as_ref().map(|c| json!({ "conversation": c }).to_string());

    let author_bookings: Vec<BookingRow> =
        match paging::query_all(host, BOOKINGS, filter.clone()).await {
            Ok(b) => b,
            Err(e) => return Err(Response::internal_error(e)),
        };

    for b in author_bookings {
        if let Some(st) = input.state
            && b.state != st
        {
            continue;
        }
        if let Ok(Some(agr)) = get_row::<AgreementRow, _>(host, AGREEMENTS, &b.agreement).await
            && (owner == agr.quote_author_did || owner == agr.counterparty_did)
        {
            let role = if owner == agr.quote_author_did {
                DealRole::QuoteAuthor
            } else {
                DealRole::Counterparty
            };
            views.push(booking_view(
                &b.agreement,
                Some(&b.snapshot),
                Some(&b.progress_record_id),
                Some(BookingWriter::Mine),
                &agr,
                role,
            ));
        }
    }

    if views.is_empty() {
        let counterparty_progress: Vec<ProgressRow> =
            match paging::query_all(host, PROGRESS, filter).await {
                Ok(pr) => pr,
                Err(e) => return Err(Response::internal_error(e)),
            };
        for pr in counterparty_progress {
            if let Some(st) = input.state
                && pr.snapshot.state != st
            {
                continue;
            }
            if let Ok(Some(agr)) = get_row::<AgreementRow, _>(host, AGREEMENTS, &pr.agreement).await
                && (owner == agr.quote_author_did || owner == agr.counterparty_did)
            {
                let role = if owner == agr.quote_author_did {
                    DealRole::QuoteAuthor
                } else {
                    DealRole::Counterparty
                };
                views.push(booking_view(
                    &pr.agreement,
                    Some(&pr.snapshot),
                    Some(&pr.record_id),
                    Some(BookingWriter::Peer),
                    &agr,
                    role,
                ));
            }
        }
    }

    let offset = usize::try_from(input.offset).unwrap_or(usize::MAX);
    let limit = usize::try_from(input.limit).unwrap_or(usize::MAX);
    let paged: Vec<BookingView> = views.into_iter().skip(offset).take(limit).collect();
    Ok(BookingListOutput { bookings: paged })
}
