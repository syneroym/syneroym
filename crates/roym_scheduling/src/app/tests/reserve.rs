use syneroym_roym_core::method::scheduling::{MAX_SLOT_CAPACITY, ReservationState, code};

use super::{NOW, code_of, host, key, reserve_input, row_ids, slot};
use crate::app::reservation::{self, Claim, SlotSeats, rows::Row};

#[tokio::test]
async fn the_last_seat_goes_to_one_of_two_reserves() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    let first = reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await;
    let second = reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), NOW).await;
    assert_eq!(first.unwrap().state, ReservationState::Held);
    assert_eq!(code_of(&second.unwrap_err()), code::SLOT_TAKEN);
    assert_eq!(row_ids(&host, "seat:").await.len(), 1);
}

#[tokio::test]
async fn two_reserves_of_one_key_return_one_reservation() {
    let host = host();
    let slot_id = slot(&host, 2).await;
    // The second caller read the seats before the first caller's claim.
    let stale = reservation::read_seats(&host, &slot_id).await.unwrap();
    let first =
        reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    let slot_seats = SlotSeats { slot_id: &slot_id, capacity: 2, seats: &stale };
    let claim = reservation::claim(&host, &key("agr-a", "c1"), 1, &slot_seats, NOW).await.unwrap();
    let Claim::Held(second) = claim else { panic!("expected the same reservation, got {claim:?}") };
    assert_eq!(second, first);
    assert_eq!(row_ids(&host, "rsv:").await.len(), 1);
    assert_eq!(row_ids(&host, "seat:").await.len(), 1);
}

#[tokio::test]
async fn a_stale_seat_read_cannot_take_a_held_seat() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    let stale = reservation::read_seats(&host, &slot_id).await.unwrap();
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    let slot_seats = SlotSeats { slot_id: &slot_id, capacity: 1, seats: &stale };
    let claim = reservation::claim(&host, &key("agr-b", "c1"), 1, &slot_seats, NOW).await.unwrap();
    assert!(matches!(claim, Claim::Full), "expected a full slot, got {claim:?}");
    assert_eq!(row_ids(&host, "rsv:").await.len(), 1);
}

#[tokio::test]
async fn a_seat_query_counts_only_seat_rows() {
    let host = host();
    let slot_id = slot(&host, 2).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    // The `rsv:` row also has a `slot_id`; it must not count as a seat.
    let second = reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), NOW).await;
    assert_eq!(second.unwrap().state, ReservationState::Held);
    let third = reservation::reserve_at(&host, reserve_input("agr-c", "c1", &slot_id), NOW).await;
    assert_eq!(code_of(&third.unwrap_err()), code::SLOT_TAKEN);
}

#[tokio::test]
async fn a_repeat_reserve_returns_the_same_reservation() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    let first = reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await;
    let again = reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await;
    assert_eq!(first.unwrap(), again.unwrap());
    assert_eq!(row_ids(&host, "seat:").await.len(), 1);
}

#[tokio::test]
async fn a_repeat_reserve_finds_a_claim_whose_pointer_was_lost() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    host.crash_on_write("reservations", "key:");
    let crashed = reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await;
    assert!(crashed.is_err());
    assert!(host.was_crashed_write_hit());
    host.restart();
    let again =
        reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    assert_eq!(again.state, ReservationState::Held);
    assert_eq!(row_ids(&host, "rsv:").await.len(), 1);
    assert_eq!(row_ids(&host, "seat:").await.len(), 1);
}

#[tokio::test]
async fn a_reserve_after_release_takes_a_new_generation() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    reservation::release_at(&host, key("agr-a", "c1"), NOW).await.unwrap();
    let again = reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await;
    assert_eq!(again.unwrap().state, ReservationState::Held);
    // Two reservations, two rounds of the one seat; nothing was deleted.
    assert_eq!(row_ids(&host, "rsv:").await.len(), 2);
    assert_eq!(row_ids(&host, "seat:").await.len(), 2);
}

#[tokio::test]
async fn capacity_is_capped_at_the_most_seats_a_slot_has() {
    let host = host();
    let slot_id = slot(&host, MAX_SLOT_CAPACITY + 10).await;
    for n in 0..MAX_SLOT_CAPACITY {
        let agreement = format!("agr-{n}");
        let held =
            reservation::reserve_at(&host, reserve_input(&agreement, "c1", &slot_id), NOW).await;
        assert_eq!(held.unwrap().state, ReservationState::Held);
    }
    let over = reservation::reserve_at(&host, reserve_input("agr-over", "c1", &slot_id), NOW).await;
    assert_eq!(code_of(&over.unwrap_err()), code::SLOT_TAKEN);
}

#[tokio::test]
async fn capacity_0_is_slot_unavailable() {
    let host = host();
    let slot_id = slot(&host, 0).await;
    let refused = reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await;
    assert_eq!(code_of(&refused.unwrap_err()), code::SLOT_UNAVAILABLE);
}

#[tokio::test]
async fn a_missing_slot_is_slot_unavailable() {
    let host = host();
    let refused =
        reservation::reserve_at(&host, reserve_input("agr-a", "c1", "slot_nowhere"), NOW).await;
    assert_eq!(code_of(&refused.unwrap_err()), code::SLOT_UNAVAILABLE);
}

#[tokio::test]
async fn a_reserve_without_a_slot_id_is_refused() {
    let host = host();
    let mut input = reserve_input("agr-a", "c1", "slot_x");
    input.details.data = serde_json::json!({ "listing_id": "lst_demo" });
    let refused = reservation::reserve_at(&host, input, NOW).await;
    assert!(refused.is_err());
}

#[tokio::test]
async fn a_reserve_that_was_refused_leaves_no_row() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    let before = (row_ids(&host, "rsv:").await, row_ids(&host, "seat:").await);
    let refused = reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), NOW).await;
    assert!(refused.is_err());
    assert_eq!(before, (row_ids(&host, "rsv:").await, row_ids(&host, "seat:").await));
}

#[test]
fn the_claim_lists_the_reservation_row_before_its_seat_row() {
    let (rows, _) = reservation::claim_rows(&key("agr-a", "c1"), "rsv_x", 1, "slot_y", (2, 3), NOW);
    let ids: Vec<&str> = rows.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["rsv:rsv_x", "seat:slot_y:2:3"]);
    assert!(matches!(rows[0].1, Row::Rsv(_)) && matches!(rows[1].1, Row::Seat(_)));
}
