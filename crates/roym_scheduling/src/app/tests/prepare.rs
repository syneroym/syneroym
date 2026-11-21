use serde_json::json;
use syneroym_roym_core::{
    method::scheduling::{PrepareInput, code},
    transaction::TimeWindow,
};

use super::{LISTING, SLOT_END, SLOT_START, code_of, details, host, slot};
use crate::app::prepare;

fn input(slot_id: &str, window: Option<TimeWindow>, listing: Option<&str>) -> PrepareInput {
    PrepareInput { details: details(slot_id), window, listing_id: listing.map(str::to_string) }
}

#[tokio::test]
async fn prepare_fills_the_window_from_the_slot() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    let out = prepare::prepare(&host, input(&slot_id, None, Some(LISTING))).await.unwrap();
    assert_eq!(out.window, TimeWindow { earliest_secs: SLOT_START, latest_secs: SLOT_END });
}

#[tokio::test]
async fn prepare_accepts_the_slots_own_window() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    let window = TimeWindow { earliest_secs: SLOT_START, latest_secs: SLOT_END };
    let out = prepare::prepare(&host, input(&slot_id, Some(window), Some(LISTING))).await.unwrap();
    assert_eq!(out.window, window);
}

#[tokio::test]
async fn prepare_refuses_a_slot_that_does_not_exist() {
    let host = host();
    let refused = prepare::prepare(&host, input("slot_nowhere", None, Some(LISTING))).await;
    let refused = refused.unwrap_err();
    assert_eq!(code_of(&refused), code::NO_SUCH_SLOT);
    assert_eq!(refused.error.unwrap().message, "no-such-slot");
}

#[tokio::test]
async fn prepare_refuses_a_slot_of_another_listing() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    let refused = prepare::prepare(&host, input(&slot_id, None, Some("lst_other"))).await;
    let refused = refused.unwrap_err();
    assert_eq!(code_of(&refused), code::SLOT_NOT_IN_LISTING);
    assert_eq!(refused.error.unwrap().message, "slot-not-in-listing");
}

#[tokio::test]
async fn prepare_refuses_details_that_name_another_listing_than_the_terms() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    let mut given = input(&slot_id, None, Some("lst_terms"));
    given.details.data = json!({ "listing_id": LISTING, "slot_id": slot_id });
    let refused = prepare::prepare(&host, given).await.unwrap_err();
    assert_eq!(code_of(&refused), code::SLOT_NOT_IN_LISTING);
}

#[tokio::test]
async fn prepare_refuses_a_window_that_is_not_the_slots() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    let window = TimeWindow { earliest_secs: SLOT_START, latest_secs: SLOT_END + 60 };
    let refused = prepare::prepare(&host, input(&slot_id, Some(window), Some(LISTING))).await;
    let refused = refused.unwrap_err();
    assert_eq!(code_of(&refused), code::SCHEDULE_DIFFERS_FROM_SLOT);
    assert_eq!(refused.error.unwrap().message, "schedule-differs-from-slot");
}

#[tokio::test]
async fn prepare_uses_the_listing_of_the_details_when_the_terms_have_none() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    let out = prepare::prepare(&host, input(&slot_id, None, None)).await.unwrap();
    assert_eq!(out.window.earliest_secs, SLOT_START);
}
