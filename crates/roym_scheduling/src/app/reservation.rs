//! The reserve, confirm and release hooks.
//!
//! A reservation holds one seat of one slot for one (agreement, commitment)
//! pair. Every state change is a unique insert, so of two calls that race,
//! only one writes, and no row is ever deleted. The rows are in [`rows`].
//!
//! A reservation of this build never expires: only `release` ends it.

use std::{collections::BTreeMap, fmt};

use serde_json::Value;
use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    clock,
    deal::Details,
    envelope::Response,
    method::{
        Reply,
        scheduling::{
            MAX_SLOT_CAPACITY, ReleaseOutput, ReservationKey, ReservationState, ReservationView,
            ReserveInput, SlotKeyInput, SlotRow, code,
        },
    },
    paging,
};

use self::rows::{
    Decided, Decision, Free, FreeWhy, Key, Row, Rsv, Seat, decided_id, free_id, key_id,
    reservation_id, rsv_id, seat_id,
};
use super::{RESERVATIONS, availability, ensure_availability, ensure_reservations, refuse};

pub(crate) mod rows;

/// How often `reserve` starts again after it meets a reservation of its own
/// key that was taken and freed while it was working. One more try is enough
/// in practice; the limit only stops a loop that cannot end.
const MAX_ATTEMPTS: u32 = 3;

fn storage(e: impl fmt::Display) -> Response {
    Response::internal_error(e.to_string())
}

fn corrupt(id: &str) -> Response {
    Response::internal_error(format!("reservation row '{id}' has an unexpected kind"))
}

async fn get_row<H: AppHost>(host: &H, id: &str) -> Result<Option<Row>, Response> {
    match AppDataLayer::get(host, RESERVATIONS.to_string(), id.to_string()).await {
        Ok(Some(row)) => serde_json::from_slice(&row.payload).map(Some).map_err(storage),
        Ok(None) => Ok(None),
        Err(e) => Err(storage(e)),
    }
}

async fn put_row<H: AppHost>(host: &H, id: String, row: &Row) -> Result<(), Response> {
    let payload = serde_json::to_vec(row).map_err(storage)?;
    AppDataLayer::put(host, RESERVATIONS.to_string(), RecordWriteValue { id, payload })
        .await
        .map_err(storage)
}

/// Creates every row or none. `Some(id)` names the first row of the list
/// that already exists; the caller never decides by that id alone.
async fn create_rows<H: AppHost>(
    host: &H,
    rows: Vec<(String, Row)>,
) -> Result<Option<String>, Response> {
    let mut values = Vec::with_capacity(rows.len());
    for (id, row) in rows {
        values.push(RecordWriteValue { id, payload: serde_json::to_vec(&row).map_err(storage)? });
    }
    AppDataLayer::create(host, RESERVATIONS.to_string(), values).await.map_err(storage)
}

fn slot_id_of(details: &Details) -> Result<&str, Response> {
    details
        .data
        .get("slot_id")
        .and_then(Value::as_str)
        .ok_or_else(|| Response::invalid_params("details.data requires slot_id"))
}

/// The newest reservation of a key. `generation` is 0 when there is none.
struct Current {
    generation: u64,
    rsv: Option<(String, Rsv)>,
}

/// Reads the pointer, then follows any newer generation. That finds a
/// reservation whose claim worked but whose pointer write was lost in a
/// crash.
async fn current<H: AppHost>(host: &H, key: &ReservationKey) -> Result<Current, Response> {
    let pointer = key_id(&key.agreement, &key.commitment_id);
    let mut generation = match get_row(host, &pointer).await? {
        Some(Row::Key(k)) => k.generation,
        Some(_) => return Err(corrupt(&pointer)),
        None => 0,
    };
    loop {
        let next = reservation_id(&key.agreement, &key.commitment_id, generation + 1)?;
        if get_row(host, &rsv_id(&next)).await?.is_none() {
            break;
        }
        generation += 1;
    }
    if generation == 0 {
        return Ok(Current { generation, rsv: None });
    }
    let r = reservation_id(&key.agreement, &key.commitment_id, generation)?;
    match get_row(host, &rsv_id(&r)).await? {
        Some(Row::Rsv(rsv)) => Ok(Current { generation, rsv: Some((r, rsv)) }),
        _ => Err(corrupt(&rsv_id(&r))),
    }
}

async fn is_free<H: AppHost>(host: &H, reservation: &str) -> Result<bool, Response> {
    Ok(get_row(host, &free_id(reservation)).await?.is_some())
}

/// The reservation of a key that is still holding its seat.
async fn live<H: AppHost>(host: &H, current: Current) -> Result<Option<(String, Rsv)>, Response> {
    match current.rsv {
        Some((r, rsv)) if !is_free(host, &r).await? => Ok(Some((r, rsv))),
        _ => Ok(None),
    }
}

async fn view<H: AppHost>(host: &H, r: &str, rsv: &Rsv) -> Reply<ReservationView> {
    let state = match get_row(host, &decided_id(r)).await? {
        Some(Row::Decided(Decided { state: Decision::Confirmed, .. })) => {
            ReservationState::Confirmed
        }
        _ => ReservationState::Held,
    };
    Ok(ReservationView {
        agreement: rsv.agreement.clone(),
        commitment_id: rsv.commitment_id.clone(),
        state,
        expires_at_secs: rsv.expires_at_secs,
    })
}

/// The occupant of each seat of one slot: the row with the highest round.
#[derive(Debug, Default)]
pub(crate) struct Seats {
    top: BTreeMap<u32, Seat>,
}

impl Seats {
    fn push(&mut self, seat: Seat) {
        let newer = self.top.get(&seat.seat).is_none_or(|held| held.round < seat.round);
        if newer {
            self.top.insert(seat.seat, seat);
        }
    }
}

/// Every seat row of one slot, all rounds. The `rsv:` rows carry the slot id
/// too, so the query also filters on `kind`.
pub(crate) async fn read_seats<H: AppHost>(host: &H, slot_id: &str) -> Result<Seats, Response> {
    let filter = serde_json::json!({ "kind": "seat", "slot_id": slot_id }).to_string();
    let found: Vec<Row> = paging::query_all(host, RESERVATIONS, Some(filter))
        .await
        .map_err(Response::internal_error)?;
    let mut seats = Seats::default();
    for row in found {
        if let Row::Seat(seat) = row {
            seats.push(seat);
        }
    }
    Ok(seats)
}

/// How a claim ended.
#[derive(Debug)]
pub(crate) enum Claim {
    /// This call took a seat, or found its own key's reservation.
    Held(ReservationView),
    /// Every seat is held.
    Full,
    /// The key's reservation was taken and freed while this call worked.
    Again,
}

/// What a claim needs to know about the slot.
pub(crate) struct SlotSeats<'a> {
    pub slot_id: &'a str,
    pub capacity: u32,
    pub seats: &'a Seats,
}

/// The rows that take one seat in one round. `rsv:` is first: when any id of a
/// `create` exists, the id it names is the first one of the list that exists,
/// so a second call with the same key is told about the reservation and not
/// about the seat.
pub(crate) fn claim_rows(
    key: &ReservationKey,
    r: &str,
    generation: u64,
    slot_id: &str,
    (seat, round): (u32, u32),
    now: u64,
) -> (Vec<(String, Row)>, Rsv) {
    let rsv = Rsv {
        agreement: key.agreement.clone(),
        commitment_id: key.commitment_id.clone(),
        generation,
        slot_id: slot_id.to_string(),
        seat,
        round,
        created_at_secs: now,
        expires_at_secs: None,
    };
    let held = Seat { reservation: r.to_string(), slot_id: slot_id.to_string(), seat, round };
    let rows =
        vec![(rsv_id(r), Row::Rsv(rsv.clone())), (seat_id(slot_id, seat, round), Row::Seat(held))];
    (rows, rsv)
}

/// Tries each seat in turn. Of two calls for one seat, only one `create`
/// works. `rsv:` is first in the list, so a second call with the same key
/// gets the `rsv:` id back and not the seat id.
pub(crate) async fn claim<H: AppHost>(
    host: &H,
    key: &ReservationKey,
    generation: u64,
    slot: &SlotSeats<'_>,
    now: u64,
) -> Result<Claim, Response> {
    let r = reservation_id(&key.agreement, &key.commitment_id, generation)?;
    for n in 1..=slot.capacity.min(MAX_SLOT_CAPACITY) {
        let occupant = slot.seats.top.get(&n);
        if let Some(held) = occupant
            && !is_free(host, &held.reservation).await?
        {
            continue;
        }
        let round = occupant.map_or(0, |s| s.round) + 1;
        let (rows, rsv) = claim_rows(key, &r, generation, slot.slot_id, (n, round), now);
        if create_rows(host, rows).await?.is_none() {
            put_row(
                host,
                key_id(&key.agreement, &key.commitment_id),
                &Row::Key(Key { generation }),
            )
            .await?;
            return Ok(Claim::Held(view(host, &r, &rsv).await?));
        }
        // Never decide by the id alone: read the key again.
        let now_current = current(host, key).await?;
        let found_generation = now_current.generation;
        if let Some((other, other_rsv)) = live(host, now_current).await? {
            let pointer = Key { generation: found_generation };
            put_row(host, key_id(&key.agreement, &key.commitment_id), &Row::Key(pointer)).await?;
            return Ok(Claim::Held(view(host, &other, &other_rsv).await?));
        }
        if found_generation >= generation {
            return Ok(Claim::Again);
        }
    }
    Ok(Claim::Full)
}

async fn load_slot<H: AppHost>(host: &H, slot_id: &str) -> Result<Option<SlotRow>, Response> {
    availability::get(host, SlotKeyInput { slot_id: slot_id.to_string() }).await
}

pub(crate) async fn reserve<H: AppHost>(host: &H, input: ReserveInput) -> Reply<ReservationView> {
    reserve_at(host, input, clock::now_secs()).await
}

pub(crate) async fn reserve_at<H: AppHost>(
    host: &H,
    input: ReserveInput,
    now: u64,
) -> Reply<ReservationView> {
    let slot_id = slot_id_of(&input.details)?.to_string();
    ensure_reservations(host).await.map_err(Response::internal_error)?;
    ensure_availability(host).await.map_err(Response::internal_error)?;
    let key = ReservationKey { agreement: input.agreement, commitment_id: input.commitment_id };
    for _ in 0..MAX_ATTEMPTS {
        let present = current(host, &key).await?;
        let generation = present.generation;
        if let Some((r, rsv)) = live(host, present).await? {
            return view(host, &r, &rsv).await;
        }
        let capacity = match load_slot(host, &slot_id).await? {
            Some(slot) if slot.capacity > 0 => slot.capacity,
            _ => return Err(refuse(code::SLOT_UNAVAILABLE)),
        };
        let seats = read_seats(host, &slot_id).await?;
        let slot = SlotSeats { slot_id: &slot_id, capacity, seats: &seats };
        match claim(host, &key, generation + 1, &slot, now).await? {
            Claim::Held(held) => return Ok(held),
            Claim::Full => return Err(refuse(code::SLOT_TAKEN)),
            Claim::Again => {}
        }
    }
    Err(Response::internal_error("a reservation could not be taken: too many races"))
}

pub(crate) async fn confirm<H: AppHost>(host: &H, key: ReservationKey) -> Reply<ReservationView> {
    confirm_at(host, key, clock::now_secs()).await
}

/// Confirms the reservation that exists. It never takes a seat: after a
/// release it is refused, also when the seat is free again.
pub(crate) async fn confirm_at<H: AppHost>(
    host: &H,
    key: ReservationKey,
    now: u64,
) -> Reply<ReservationView> {
    ensure_reservations(host).await.map_err(Response::internal_error)?;
    let Some((r, rsv)) = current(host, &key).await?.rsv else {
        return Err(refuse(code::RESERVATION_UNKNOWN));
    };
    let decided = match get_row(host, &decided_id(&r)).await? {
        Some(Row::Decided(d)) => d,
        Some(_) => return Err(corrupt(&decided_id(&r))),
        None => {
            if is_free(host, &r).await? {
                return Err(refuse(code::RESERVATION_LOST));
            }
            let mine = Decided { state: Decision::Confirmed, at_secs: now };
            if create_rows(host, vec![(decided_id(&r), Row::Decided(mine.clone()))])
                .await?
                .is_none()
            {
                mine
            } else {
                match get_row(host, &decided_id(&r)).await? {
                    Some(Row::Decided(d)) => d,
                    _ => return Err(corrupt(&decided_id(&r))),
                }
            }
        }
    };
    match decided.state {
        Decision::Confirmed => view(host, &r, &rsv).await,
        Decision::Lapsed => Err(refuse(code::RESERVATION_LOST)),
    }
}

pub(crate) async fn release<H: AppHost>(host: &H, key: ReservationKey) -> Reply<ReleaseOutput> {
    release_at(host, key, clock::now_secs()).await
}

/// Frees the seat of a reservation, also a confirmed one. A key that is
/// unknown or already free answers `released: false`.
pub(crate) async fn release_at<H: AppHost>(
    host: &H,
    key: ReservationKey,
    now: u64,
) -> Reply<ReleaseOutput> {
    ensure_reservations(host).await.map_err(Response::internal_error)?;
    let Some((r, _)) = current(host, &key).await?.rsv else {
        return Ok(ReleaseOutput { released: false });
    };
    let free = Free { why: FreeWhy::Released, at_secs: now };
    let created = create_rows(host, vec![(free_id(&r), Row::Free(free))]).await?.is_none();
    Ok(ReleaseOutput { released: created })
}
