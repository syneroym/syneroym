use syneroym_roym_core::method::scheduling::{ReservationState, code};

use super::{NOW, code_of, host, key, reserve_input, row_ids, slot};
use crate::app::reservation;

#[tokio::test]
async fn confirm_of_an_unknown_key_is_reservation_unknown() {
    let host = host();
    let refused = reservation::confirm_at(&host, key("agr-none", "c1"), NOW).await;
    assert_eq!(code_of(&refused.unwrap_err()), code::RESERVATION_UNKNOWN);
}

#[tokio::test]
async fn confirm_makes_the_reservation_confirmed_and_a_repeat_succeeds() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    let first = reservation::confirm_at(&host, key("agr-a", "c1"), NOW).await.unwrap();
    let again = reservation::confirm_at(&host, key("agr-a", "c1"), NOW + 9).await.unwrap();
    assert_eq!(first.state, ReservationState::Confirmed);
    assert_eq!(again, first);
    // A repeat reserve now reports the confirmed state.
    let repeat = reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await;
    assert_eq!(repeat.unwrap().state, ReservationState::Confirmed);
}

#[tokio::test]
async fn confirm_after_a_release_is_reservation_lost_and_takes_no_seat() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    reservation::release_at(&host, key("agr-a", "c1"), NOW).await.unwrap();
    let seats_before = row_ids(&host, "seat:").await;
    let refused = reservation::confirm_at(&host, key("agr-a", "c1"), NOW + 1).await;
    assert_eq!(code_of(&refused.unwrap_err()), code::RESERVATION_LOST);
    assert_eq!(row_ids(&host, "seat:").await, seats_before);
    // The seat is free, and a booking by another agreement can still take it.
    let taker = reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), NOW).await;
    assert_eq!(taker.unwrap().state, ReservationState::Held);
}

#[tokio::test]
async fn confirm_never_creates_a_seat_row() {
    let host = host();
    let slot_id = slot(&host, 2).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), NOW).await.unwrap();
    reservation::release_at(&host, key("agr-b", "c1"), NOW).await.unwrap();
    let seats = row_ids(&host, "seat:").await;
    let rsvs = row_ids(&host, "rsv:").await;
    // Every kind of confirm: a held one, a repeat, an unknown one, a released one.
    let _ = reservation::confirm_at(&host, key("agr-a", "c1"), NOW).await;
    let _ = reservation::confirm_at(&host, key("agr-a", "c1"), NOW).await;
    let _ = reservation::confirm_at(&host, key("agr-none", "c1"), NOW).await;
    let _ = reservation::confirm_at(&host, key("agr-b", "c1"), NOW).await;
    assert_eq!(row_ids(&host, "seat:").await, seats);
    assert_eq!(row_ids(&host, "rsv:").await, rsvs);
}

#[tokio::test]
async fn a_confirm_after_a_confirm_and_a_release_answers_its_stored_state() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    reservation::confirm_at(&host, key("agr-a", "c1"), NOW).await.unwrap();
    reservation::release_at(&host, key("agr-a", "c1"), NOW).await.unwrap();
    let again = reservation::confirm_at(&host, key("agr-a", "c1"), NOW + 1).await.unwrap();
    assert_eq!(again.state, ReservationState::Confirmed);
}
