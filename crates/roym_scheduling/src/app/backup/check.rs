//! The checks of `scheduling.import`. They run before any row is written.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use syneroym_app_host::{AppDataLayer, AppHost};
use syneroym_roym_core::{envelope::Response, listing, method::scheduling::SlotRow};

use crate::app::{
    RESERVATIONS,
    reservation::rows::{Row, Rsv, Seat, key_id, reservation_id, rsv_id, seat_id},
};

/// A refused bundle: `invalid_params` with a message that names the row.
fn bad(message: impl Into<String>) -> Response {
    Response::invalid_params(message)
}

fn record_parts(record: &Value) -> Result<(&str, &Value), Response> {
    let id = record.get("id").and_then(Value::as_str).ok_or_else(|| bad("record missing id"))?;
    let payload = record.get("payload").ok_or_else(|| bad("record missing payload"))?;
    Ok((id, payload))
}

/// Slot rows: each parses, and its id is the one its listing and window give.
pub(super) fn availability_rows(records: &[Value]) -> Result<Vec<(String, Value)>, Response> {
    let mut out = Vec::with_capacity(records.len());
    for record in records {
        let (id, payload) = record_parts(record)?;
        let slot: SlotRow = serde_json::from_value(payload.clone())
            .map_err(|e| bad(format!("availability record '{id}' does not parse: {e}")))?;
        let derived = listing::derive_slot_id(&slot.listing_id, slot.start_secs, slot.end_secs)
            .map_err(|e| Response::internal_error(e.to_string()))?;
        if slot.slot_id != id || slot.slot_id != derived {
            return Err(bad(format!("availability record '{id}' has the wrong slot id")));
        }
        out.push((id.to_string(), payload.clone()));
    }
    Ok(out)
}

/// What a bundle's reservation rows say, sorted into kinds.
#[derive(Default)]
struct Parsed {
    rsvs: BTreeMap<String, (Rsv, Value)>,
    seats: BTreeMap<String, (Seat, Value)>,
    keys: Vec<(String, u64, Value)>,
    after: Vec<(String, Value)>,
}

fn parse_rows(records: &[Value]) -> Result<Parsed, Response> {
    let mut parsed = Parsed::default();
    let mut ids = BTreeSet::new();
    for record in records {
        let (id, payload) = record_parts(record)?;
        if !ids.insert(id.to_string()) {
            return Err(bad(format!("reservation record '{id}' is listed twice")));
        }
        let row: Row = serde_json::from_value(payload.clone())
            .map_err(|e| bad(format!("reservation record '{id}' does not parse: {e}")))?;
        let wrong_prefix =
            || bad(format!("reservation record '{id}' has an id that does not fit its kind"));
        match row {
            Row::Rsv(rsv) => {
                let r = id.strip_prefix("rsv:").ok_or_else(wrong_prefix)?;
                parsed.rsvs.insert(r.to_string(), (rsv, payload.clone()));
            }
            Row::Seat(seat) => {
                parsed.seats.insert(id.to_string(), (seat, payload.clone()));
            }
            Row::Key(key) => parsed.keys.push((id.to_string(), key.generation, payload.clone())),
            Row::Decided(_) => {
                id.strip_prefix("decided:").ok_or_else(wrong_prefix)?;
                parsed.after.push((id.to_string(), payload.clone()));
            }
            Row::Free(_) => {
                id.strip_prefix("free:").ok_or_else(wrong_prefix)?;
                parsed.after.push((id.to_string(), payload.clone()));
            }
        }
    }
    Ok(parsed)
}

/// Reservation rows, in the order they are written: reservations with their
/// seats first, so a stopped import never leaves a seat with no reservation.
pub(super) fn reservation_rows(records: &[Value]) -> Result<Vec<(String, Value)>, Response> {
    let parsed = parse_rows(records)?;
    check_reservations(&parsed)?;
    check_decisions(&parsed)?;
    check_keys(&parsed)?;
    let mut out = Vec::with_capacity(records.len());
    for (r, (_, payload)) in &parsed.rsvs {
        out.push((rsv_id(r), payload.clone()));
    }
    for (id, (_, payload)) in &parsed.seats {
        out.push((id.clone(), payload.clone()));
    }
    out.extend(parsed.keys.into_iter().map(|(id, _, payload)| (id, payload)));
    out.extend(parsed.after);
    Ok(out)
}

/// Each reservation has the id its key and generation give, and the seat row
/// that names it back. Each seat row names a reservation of the bundle.
fn check_reservations(parsed: &Parsed) -> Result<(), Response> {
    for (r, (rsv, _)) in &parsed.rsvs {
        let derived = reservation_id(&rsv.agreement, &rsv.commitment_id, rsv.generation)?;
        if &derived != r {
            return Err(bad(format!("reservation '{r}' is not the id of its key and generation")));
        }
        let seat_row = seat_id(&rsv.slot_id, rsv.seat, rsv.round);
        let names_back = parsed.seats.get(&seat_row).is_some_and(|(seat, _)| {
            &seat.reservation == r
                && seat.slot_id == rsv.slot_id
                && seat.seat == rsv.seat
                && seat.round == rsv.round
        });
        if !names_back {
            return Err(bad(format!("reservation '{r}' has no seat row that names it")));
        }
    }
    for (id, (seat, _)) in &parsed.seats {
        if id != &seat_id(&seat.slot_id, seat.seat, seat.round) {
            return Err(bad(format!("seat row '{id}' has an id that does not fit its seat")));
        }
        let held = parsed.rsvs.get(&seat.reservation).is_some_and(|(rsv, _)| {
            rsv.slot_id == seat.slot_id && rsv.seat == seat.seat && rsv.round == seat.round
        });
        if !held {
            return Err(bad(format!("seat row '{id}' names no reservation of the bundle")));
        }
    }
    Ok(())
}

fn check_decisions(parsed: &Parsed) -> Result<(), Response> {
    for (id, _) in &parsed.after {
        let r =
            id.strip_prefix("decided:").or_else(|| id.strip_prefix("free:")).unwrap_or_default();
        if !parsed.rsvs.contains_key(r) {
            return Err(bad(format!(
                "reservation record '{id}' names no reservation of the bundle"
            )));
        }
    }
    Ok(())
}

/// A pointer names a reservation of the bundle, under the key it is for.
fn check_keys(parsed: &Parsed) -> Result<(), Response> {
    for (id, generation, _) in &parsed.keys {
        let named = parsed.rsvs.values().any(|(rsv, _)| {
            rsv.generation == *generation
                && id == &key_id(&rsv.agreement, &rsv.commitment_id)
                && reservation_id(&rsv.agreement, &rsv.commitment_id, *generation)
                    .is_ok_and(|r| parsed.rsvs.contains_key(&r))
        });
        if !named {
            return Err(bad(format!("pointer '{id}' names no reservation of the bundle")));
        }
    }
    Ok(())
}

/// Import merges rows into the store. A reservation or seat row that exists
/// with other content is refused, so a bundle cannot move a held seat to
/// another reservation.
pub(super) async fn refuse_changed_rows<H: AppHost>(
    host: &H,
    rows: &[(String, Value)],
) -> Result<(), Response> {
    for (id, payload) in rows {
        if !id.starts_with("rsv:") && !id.starts_with("seat:") {
            continue;
        }
        let stored = AppDataLayer::get(host, RESERVATIONS.to_string(), id.clone())
            .await
            .map_err(|e| Response::internal_error(e.to_string()))?;
        let Some(stored) = stored else { continue };
        let same = serde_json::from_slice::<Value>(&stored.payload).is_ok_and(|v| &v == payload);
        if !same {
            return Err(bad("reservation-differs")
                .with_data(serde_json::json!({ "code": "reservation-differs", "id": id })));
        }
    }
    Ok(())
}
