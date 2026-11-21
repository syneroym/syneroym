//! Method input and output types of the scheduling service: the slot store
//! and the reservation hooks that the deal core calls.
//!
//! The hook types are generic. They carry a commitment's `details` and
//! `window` and never a slot id, so the deal core reads no slot field.

use serde::{Deserialize, Serialize};

use crate::{deal::Details, transaction::TimeWindow};

/// The most seats one slot can have. A larger stored capacity counts as this
/// many, so a reserve reads a bounded number of seats.
pub const MAX_SLOT_CAPACITY: u32 = 64;

/// Input for methods identifying a slot by slot_id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotKeyInput {
    pub slot_id: String,
}

/// A stored availability slot row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotRow {
    pub slot_id: String,
    pub listing_id: String,
    pub start_secs: u64,
    pub end_secs: u64,
    pub capacity: u32,
}

/// Names one reservation. `agreement` is the quote version's record id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReservationKey {
    pub agreement: String,
    pub commitment_id: String,
}

/// Input of the `prepare` hook: what the author gave for one commitment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareInput {
    pub details: Details,
    #[serde(default)]
    pub window: Option<TimeWindow>,
    /// The listing id of the quote's terms, when it has one.
    #[serde(default)]
    pub listing_id: Option<String>,
}

/// Output of the `prepare` hook: the window the quote will carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareOutput {
    pub window: TimeWindow,
}

/// Input of the `reserve` hook. It has no `window`: the slot id already
/// names its window, and the schema check has proved that it does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReserveInput {
    pub agreement: String,
    pub commitment_id: String,
    pub details: Details,
}

/// How far a reservation has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReservationState {
    Held,
    Confirmed,
}

/// Output of `reserve` and `confirm`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservationView {
    pub agreement: String,
    pub commitment_id: String,
    pub state: ReservationState,
    /// The time from which another booking may take the seat, unless the
    /// reservation is confirmed first.
    pub expires_at_secs: Option<u64>,
}

/// Output of `release`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseOutput {
    /// `false` when the key is unknown or already released.
    pub released: bool,
}

/// The refusal codes of the reservation hooks, in `data.code`.
pub mod code {
    pub const NO_SUCH_SLOT: &str = "no-such-slot";
    pub const SLOT_NOT_IN_LISTING: &str = "slot-not-in-listing";
    pub const SCHEDULE_DIFFERS_FROM_SLOT: &str = "schedule-differs-from-slot";
    pub const SLOT_UNAVAILABLE: &str = "slot-unavailable";
    pub const SLOT_TAKEN: &str = "slot-taken";
    pub const RESERVATION_UNKNOWN: &str = "reservation-unknown";
    pub const RESERVATION_LOST: &str = "reservation-lost";
}
