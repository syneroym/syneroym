//! Decision and seat reservation ledger for provider bookings.

use serde::{Deserialize, Serialize};
use serde_json::json;
use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::RecordWriteValue};
use syneroym_roym_core::booking::{self, BookingProgressPayload, BookingState, ConflictReason};

use super::{BOOKINGS, BookingRow, LEDGER, catalog_call, get_row, put_row};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum LedgerKind {
    Decision,
    Seat,
    Step,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LedgerRow {
    pub(crate) kind: LedgerKind,
    pub(crate) agreement: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) slot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) seat: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) step: Option<StepRow>,
    pub(crate) created_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StepRow {
    pub(crate) seq: u32,
    pub(crate) event: String,
    pub(crate) snapshot: BookingProgressPayload,
    pub(crate) envelope: String,
    pub(crate) record_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) message_id: Option<String>,
}

pub(crate) fn decision_id(agreement: &str) -> String {
    format!("decision:{agreement}")
}

pub(crate) fn seat_id(slot_id: &str, n: u32) -> String {
    format!("seat:{slot_id}:{n}")
}

pub(crate) fn step_id(agreement: &str, seq: u32) -> String {
    format!("step:{agreement}:{seq}")
}

pub(crate) enum Claim {
    Won(Box<BookingRow>),
    AlreadyDecided,
    NoSeat(ConflictReason),
}

pub(crate) async fn claim_decision<H: AppHost>(
    host: &H,
    agreement: &str,
    slot_id: Option<&str>,
    seat: Option<u32>,
    scheduled: &BookingProgressPayload,
    scheduled_env: &(String, String),
    now: u64,
) -> Result<Claim, String> {
    let dec_row = LedgerRow {
        kind: LedgerKind::Decision,
        agreement: agreement.to_string(),
        slot_id: slot_id.map(ToString::to_string),
        seat,
        step: None,
        created_at_secs: now,
    };
    let step_row = LedgerRow {
        kind: LedgerKind::Step,
        agreement: agreement.to_string(),
        slot_id: slot_id.map(ToString::to_string),
        seat,
        step: Some(StepRow {
            seq: 1,
            event: "open".to_string(),
            snapshot: scheduled.clone(),
            envelope: scheduled_env.0.clone(),
            record_id: scheduled_env.1.clone(),
            message_id: None,
        }),
        created_at_secs: now,
    };

    let rows = vec![
        RecordWriteValue {
            id: decision_id(agreement),
            payload: serde_json::to_vec(&dec_row).map_err(|e| e.to_string())?,
        },
        RecordWriteValue {
            id: step_id(agreement, 1),
            payload: serde_json::to_vec(&step_row).map_err(|e| e.to_string())?,
        },
    ];

    match AppDataLayer::create(host, LEDGER.to_string(), rows).await.map_err(|e| e.to_string())? {
        None => Ok(Claim::Won(Box::new(BookingRow {
            agreement: agreement.to_string(),
            conversation: scheduled.conversation.clone(),
            state: scheduled.state,
            slot_id: slot_id.map(ToString::to_string),
            seat,
            snapshot: scheduled.clone(),
            progress_record_id: scheduled_env.1.clone(),
            updated_at_secs: now,
        }))),
        Some(id) if id == decision_id(agreement) || id == step_id(agreement, 1) => {
            Ok(Claim::AlreadyDecided)
        }
        Some(_) => Ok(Claim::AlreadyDecided),
    }
}

pub(crate) async fn claim_seat<H: AppHost>(
    host: &H,
    q: &str,
    slot: &str,
    scheduled: &BookingProgressPayload,
    scheduled_env: &(String, String),
    now: u64,
) -> Result<Claim, String> {
    let slot_resp = catalog_call(host, "availability.get", json!({ "slot_id": slot })).await?;
    let slot_row = match slot_resp.result {
        Some(serde_json::Value::Object(map)) => map,
        _ => return Ok(Claim::NoSeat(ConflictReason::SlotUnavailable)),
    };
    let capacity = slot_row.get("capacity").and_then(serde_json::Value::as_u64).unwrap_or(0) as u32;
    if capacity == 0 {
        return Ok(Claim::NoSeat(ConflictReason::SlotUnavailable));
    }
    let capacity = std::cmp::min(capacity, booking::MAX_SLOT_CAPACITY);
    for n in 1..=capacity {
        let outcome = try_claim_seat(host, q, slot, n, scheduled, scheduled_env, now).await?;
        match outcome {
            SeatAttempt::Won(row) => return Ok(Claim::Won(row)),
            SeatAttempt::AlreadyDecided => return Ok(Claim::AlreadyDecided),
            SeatAttempt::SeatTaken => continue,
        }
    }
    Ok(Claim::NoSeat(ConflictReason::SlotTaken))
}

enum SeatAttempt {
    Won(Box<BookingRow>),
    AlreadyDecided,
    SeatTaken,
}

async fn try_claim_seat<H: AppHost>(
    host: &H,
    q: &str,
    slot: &str,
    n: u32,
    scheduled: &BookingProgressPayload,
    scheduled_env: &(String, String),
    now: u64,
) -> Result<SeatAttempt, String> {
    let dec_row = LedgerRow {
        kind: LedgerKind::Decision,
        agreement: q.to_string(),
        slot_id: Some(slot.to_string()),
        seat: Some(n),
        step: None,
        created_at_secs: now,
    };
    let seat_row = LedgerRow {
        kind: LedgerKind::Seat,
        agreement: q.to_string(),
        slot_id: Some(slot.to_string()),
        seat: Some(n),
        step: None,
        created_at_secs: now,
    };
    let step_row = LedgerRow {
        kind: LedgerKind::Step,
        agreement: q.to_string(),
        slot_id: Some(slot.to_string()),
        seat: Some(n),
        step: Some(StepRow {
            seq: 1,
            event: "open".to_string(),
            snapshot: scheduled.clone(),
            envelope: scheduled_env.0.clone(),
            record_id: scheduled_env.1.clone(),
            message_id: None,
        }),
        created_at_secs: now,
    };

    let rows = vec![
        RecordWriteValue {
            id: decision_id(q),
            payload: serde_json::to_vec(&dec_row).map_err(|e| e.to_string())?,
        },
        RecordWriteValue {
            id: seat_id(slot, n),
            payload: serde_json::to_vec(&seat_row).map_err(|e| e.to_string())?,
        },
        RecordWriteValue {
            id: step_id(q, 1),
            payload: serde_json::to_vec(&step_row).map_err(|e| e.to_string())?,
        },
    ];

    match AppDataLayer::create(host, LEDGER.to_string(), rows).await.map_err(|e| e.to_string())? {
        None => Ok(SeatAttempt::Won(Box::new(BookingRow {
            agreement: q.to_string(),
            conversation: scheduled.conversation.clone(),
            state: scheduled.state,
            slot_id: Some(slot.to_string()),
            seat: Some(n),
            snapshot: scheduled.clone(),
            progress_record_id: scheduled_env.1.clone(),
            updated_at_secs: now,
        }))),
        Some(id) if id == decision_id(q) || id == step_id(q, 1) => Ok(SeatAttempt::AlreadyDecided),
        Some(_) => Ok(SeatAttempt::SeatTaken),
    }
}

pub(crate) async fn roll_forward<H: AppHost>(host: &H, row: &mut BookingRow) -> Result<(), String> {
    let initial_seq = row.snapshot.seq;
    let mut seq = initial_seq + 1;
    loop {
        let step_key = step_id(&row.agreement, seq);
        let ledger_row: Option<LedgerRow> = get_row(host, LEDGER, &step_key).await?;
        match ledger_row.and_then(|r| r.step) {
            Some(step) => {
                row.snapshot = step.snapshot;
                row.state = row.snapshot.state;
                row.progress_record_id = step.record_id;
                seq += 1;
            }
            None => break,
        }
    }
    if seq > initial_seq + 1 {
        if row.state == BookingState::Cancelled
            && let (Some(slot), Some(seat)) = (&row.slot_id, row.seat)
        {
            let _ = AppDataLayer::delete(host, LEDGER.to_string(), seat_id(slot, seat)).await;
        }
        put_row(host, BOOKINGS, &row.agreement, row).await?;
    }
    Ok(())
}

pub(crate) async fn load_booking<H: AppHost>(
    host: &H,
    agreement: &str,
) -> Result<Option<BookingRow>, String> {
    if let Some(mut row) = get_row::<BookingRow, _>(host, BOOKINGS, agreement).await? {
        roll_forward(host, &mut row).await?;
        return Ok(Some(row));
    }
    let dec_key = decision_id(agreement);
    let dec_row: Option<LedgerRow> = get_row(host, LEDGER, &dec_key).await?;
    let Some(dec) = dec_row else {
        return Ok(None);
    };

    let mut highest_step: Option<StepRow> = None;
    let mut seq = 1;
    loop {
        let step_key = step_id(agreement, seq);
        let step_ledger: Option<LedgerRow> = get_row(host, LEDGER, &step_key).await?;
        match step_ledger.and_then(|r| r.step) {
            Some(step) => {
                highest_step = Some(step);
                seq += 1;
            }
            None => break,
        }
    }

    let Some(step) = highest_step else {
        return Ok(None);
    };

    let row = BookingRow {
        agreement: agreement.to_string(),
        conversation: step.snapshot.conversation.clone(),
        state: step.snapshot.state,
        slot_id: dec.slot_id.clone(),
        seat: dec.seat,
        snapshot: step.snapshot,
        progress_record_id: step.record_id,
        updated_at_secs: dec.created_at_secs,
    };
    if row.state == BookingState::Cancelled
        && let (Some(slot), Some(seat)) = (&row.slot_id, row.seat)
    {
        let _ = AppDataLayer::delete(host, LEDGER.to_string(), seat_id(slot, seat)).await;
    }
    put_row(host, BOOKINGS, agreement, &row).await?;
    Ok(Some(row))
}
