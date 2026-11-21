//! Test fixtures for the scheduling service's in-crate tests.

mod backup;
mod confirm;
mod lapse;
mod prepare;
mod release;
mod reserve;

use serde_json::json;
use syneroym_roym_core::{
    deal::Details,
    method::scheduling::{ReservationKey, ReserveInput},
    testing::TestHost,
};

use crate::app::availability::{self, AvailabilitySetInput, SlotInput};

pub(crate) const LISTING: &str = "lst_demo";
pub(crate) const SLOT_START: u64 = 10_000;
pub(crate) const SLOT_END: u64 = 13_600;
pub(crate) const NOW: u64 = 1_000;

pub(crate) fn host() -> TestHost {
    let host = TestHost::default();
    host.set_now(NOW);
    host
}

/// Stores one slot of the given capacity and answers its id.
pub(crate) async fn slot(host: &TestHost, capacity: u32) -> String {
    let input = AvailabilitySetInput {
        listing_id: LISTING.to_string(),
        slots: vec![SlotInput { start_secs: SLOT_START, end_secs: SLOT_END, capacity }],
    };
    let out = availability::set(host, input).await.unwrap();
    out.slot_ids[0].clone()
}

pub(crate) fn details(slot_id: &str) -> Details {
    Details {
        extension: "scheduling".to_string(),
        schema: "scheduling/slot@1".to_string(),
        data: json!({ "listing_id": LISTING, "slot_id": slot_id }),
        extra: Vec::new(),
    }
}

pub(crate) fn key(agreement: &str, commitment_id: &str) -> ReservationKey {
    ReservationKey { agreement: agreement.to_string(), commitment_id: commitment_id.to_string() }
}

pub(crate) fn reserve_input(agreement: &str, commitment_id: &str, slot_id: &str) -> ReserveInput {
    ReserveInput {
        agreement: agreement.to_string(),
        commitment_id: commitment_id.to_string(),
        details: details(slot_id),
    }
}

/// The ids of every row in the `reservations` collection that starts with
/// `prefix`.
pub(crate) async fn row_ids(host: &TestHost, prefix: &str) -> Vec<String> {
    use syneroym_roym_core::paging;

    let mut ids = Vec::new();
    let mut pages = paging::Pages::new(host, super::RESERVATIONS, None);
    while let Some(rows) = pages.next_page().await.unwrap() {
        ids.extend(rows.into_iter().map(|r| r.id).filter(|id| id.starts_with(prefix)));
    }
    ids.sort();
    ids
}

/// The `data.code` of a refusal.
pub(crate) fn code_of(response: &syneroym_roym_core::envelope::Response) -> String {
    response
        .error
        .as_ref()
        .and_then(|e| e.data.as_ref())
        .and_then(|d| d.get("code"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}
