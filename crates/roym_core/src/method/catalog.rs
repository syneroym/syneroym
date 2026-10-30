//! Shared method input and output types for catalog operations.

use serde::{Deserialize, Serialize};

use crate::listing::ListingStatus;

/// Input for methods identifying a listing by listing_id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListingKeyInput {
    pub listing_id: String,
}

/// Output of `listing.get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListingGetOutput {
    pub envelope: String,
    pub record_id: String,
    pub listing_id: String,
    pub status: ListingStatus,
    pub updated_at_secs: u64,
}

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
