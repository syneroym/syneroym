use syneroym_roym_core::method::scheduling::{ReservationState, code};

use super::{NOW, code_of, host, key, reserve_input, row_ids, slot};
use crate::app::reservation;

#[tokio::test]
async fn release_frees_the_seat_and_a_second_release_changes_nothing() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    let first = reservation::release_at(&host, key("agr-a", "c1"), NOW).await.unwrap();
    let second = reservation::release_at(&host, key("agr-a", "c1"), NOW + 5).await.unwrap();
    assert!(first.released);
    assert!(!second.released);
    assert_eq!(row_ids(&host, "free:").await.len(), 1);
    let taker = reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), NOW).await;
    assert_eq!(taker.unwrap().state, ReservationState::Held);
}

#[tokio::test]
async fn release_of_an_unknown_key_succeeds() {
    let host = host();
    let out = reservation::release_at(&host, key("agr-none", "c1"), NOW).await.unwrap();
    assert!(!out.released);
}

#[tokio::test]
async fn release_frees_a_confirmed_reservation() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    reservation::confirm_at(&host, key("agr-a", "c1"), NOW).await.unwrap();
    let out = reservation::release_at(&host, key("agr-a", "c1"), NOW).await.unwrap();
    assert!(out.released);
    let taker = reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), NOW).await;
    assert_eq!(taker.unwrap().state, ReservationState::Held);
}

#[tokio::test]
async fn a_second_release_cannot_free_a_seat_taken_again() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    reservation::release_at(&host, key("agr-a", "c1"), NOW).await.unwrap();
    reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), NOW).await.unwrap();
    // A's release runs again, as a repeat or a slow duplicate would.
    let again = reservation::release_at(&host, key("agr-a", "c1"), NOW + 1).await.unwrap();
    assert!(!again.released);
    let third = reservation::reserve_at(&host, reserve_input("agr-c", "c1", &slot_id), NOW).await;
    assert_eq!(code_of(&third.unwrap_err()), code::SLOT_TAKEN);
}
