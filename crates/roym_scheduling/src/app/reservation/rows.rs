//! The rows of the `reservations` collection, and their ids.
//!
//! No seat row is ever deleted or overwritten. A seat is taken by a new row
//! in a later round, and freed by a separate `free:` row. So no call can free
//! a seat that another reservation holds, whatever order the calls run in.

use serde::{Deserialize, Serialize};
use syneroym_roym_core::{envelope::Response, record::content_digest};

/// The prefix of a reservation id.
const RESERVATION_ID_PREFIX: &str = "rsv_";

/// One row of the `reservations` collection. The `kind` field is on the wire
/// and in the index, so the seat query can ask for seat rows only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Row {
    Rsv(Rsv),
    Seat(Seat),
    Key(Key),
    Decided(Decided),
    Free(Free),
}

/// The reservation itself, created together with its seat row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rsv {
    pub agreement: String,
    pub commitment_id: String,
    pub generation: u64,
    pub slot_id: String,
    pub seat: u32,
    pub round: u32,
    pub created_at_secs: u64,
    pub expires_at_secs: Option<u64>,
}

/// The claim on one seat in one round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seat {
    pub reservation: String,
    pub slot_id: String,
    pub seat: u32,
    pub round: u32,
}

/// A pointer to the newest generation of one (agreement, commitment) pair.
/// It is the only row that is overwritten, and it is a hint: the reader
/// checks the next generation too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Decision {
    Confirmed,
    Lapsed,
}

/// How a reservation ended its waiting. Created once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decided {
    pub state: Decision,
    pub at_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FreeWhy {
    Released,
    Lapsed,
}

/// The seat of a reservation is free again. Created once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Free {
    pub why: FreeWhy,
    pub at_secs: u64,
}

pub fn rsv_id(reservation: &str) -> String {
    format!("rsv:{reservation}")
}

pub fn seat_id(slot_id: &str, seat: u32, round: u32) -> String {
    format!("seat:{slot_id}:{seat}:{round}")
}

pub fn key_id(agreement: &str, commitment_id: &str) -> String {
    format!("key:{agreement}:{commitment_id}")
}

pub fn decided_id(reservation: &str) -> String {
    format!("decided:{reservation}")
}

pub fn free_id(reservation: &str) -> String {
    format!("free:{reservation}")
}

/// The id of the reservation for one generation of a key. It is a digest, so
/// the same key and generation always give the same id.
pub fn reservation_id(
    agreement: &str,
    commitment_id: &str,
    generation: u64,
) -> Result<String, Response> {
    content_digest(
        RESERVATION_ID_PREFIX,
        &serde_json::json!({
            "agreement": agreement,
            "commitment_id": commitment_id,
            "generation": generation,
        }),
    )
    .map_err(|e| Response::internal_error(e.to_string()))
}
