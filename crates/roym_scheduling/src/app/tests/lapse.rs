use syneroym_roym_core::method::scheduling::{ReservationState, code};

use super::{NOW, code_of, host, key, reserve_input, row_ids, slot};
use crate::app::{
    RESERVATIONS,
    reservation::{
        self, RESERVATION_TTL_SECS,
        rows::{Decision, Row},
    },
};

const DUE: u64 = NOW + RESERVATION_TTL_SECS;

async fn decision_of(host: &syneroym_roym_core::testing::TestHost, id: &str) -> Option<Decision> {
    use syneroym_app_host::AppDataLayer;

    let row = AppDataLayer::get(host, RESERVATIONS.to_string(), id.to_string()).await.unwrap()?;
    match serde_json::from_slice(&row.payload).unwrap() {
        Row::Decided(d) => Some(d.state),
        other => panic!("expected a decided row, got {other:?}"),
    }
}

#[tokio::test]
async fn a_reservation_carries_its_expiry() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    let held =
        reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    assert_eq!(held.expires_at_secs, Some(DUE));
}

#[tokio::test]
async fn an_unconfirmed_reservation_lapses_when_its_seat_is_needed() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    let taker = reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), DUE + 1)
        .await
        .unwrap();
    assert_eq!(taker.state, ReservationState::Held);
    let first = row_ids(&host, "decided:").await;
    assert_eq!(first.len(), 1, "only the lapsed reservation has a decision");
    assert_eq!(decision_of(&host, &first[0]).await, Some(Decision::Lapsed));
    assert_eq!(row_ids(&host, "free:").await.len(), 1);
    // The seat is held again in a new round, and no seat row was removed.
    assert_eq!(row_ids(&host, "seat:").await.len(), 2);
}

#[tokio::test]
async fn a_reservation_before_its_expiry_is_not_taken() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    let early = reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), DUE - 1);
    assert_eq!(code_of(&early.await.unwrap_err()), code::SLOT_TAKEN);
    assert!(row_ids(&host, "decided:").await.is_empty());
}

#[tokio::test]
async fn a_reservation_is_taken_at_its_expiry_second() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    let taker =
        reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), DUE).await.unwrap();
    assert_eq!(taker.state, ReservationState::Held);
}

#[tokio::test]
async fn a_confirmed_reservation_never_lapses() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    reservation::confirm_at(&host, key("agr-a", "c1"), NOW + 1).await.unwrap();
    let late = reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), DUE * 10);
    assert_eq!(code_of(&late.await.unwrap_err()), code::SLOT_TAKEN);
    assert!(row_ids(&host, "free:").await.is_empty());
}

#[tokio::test]
async fn confirm_after_the_expiry_succeeds_while_nobody_took_the_seat() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    let confirmed = reservation::confirm_at(&host, key("agr-a", "c1"), DUE + 500).await.unwrap();
    assert_eq!(confirmed.state, ReservationState::Confirmed);
    // The confirm closed the seat: a later booking cannot take it.
    let late = reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), DUE + 501);
    assert_eq!(code_of(&late.await.unwrap_err()), code::SLOT_TAKEN);
}

#[tokio::test]
async fn confirm_after_a_takeover_is_reservation_lost() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), DUE + 1).await.unwrap();
    let refused = reservation::confirm_at(&host, key("agr-a", "c1"), DUE + 2).await;
    assert_eq!(code_of(&refused.unwrap_err()), code::RESERVATION_LOST);
    // The taker is unharmed and can still confirm.
    let taker = reservation::confirm_at(&host, key("agr-b", "c1"), DUE + 2).await.unwrap();
    assert_eq!(taker.state, ReservationState::Confirmed);
}

#[tokio::test]
async fn confirm_after_a_release_and_the_expiry_takes_no_seat() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    reservation::release_at(&host, key("agr-a", "c1"), NOW + 1).await.unwrap();
    let seats = row_ids(&host, "seat:").await;
    // Even after the expiry, with the seat free, the confirm takes nothing.
    let refused = reservation::confirm_at(&host, key("agr-a", "c1"), DUE + 1).await;
    assert_eq!(code_of(&refused.unwrap_err()), code::RESERVATION_LOST);
    assert_eq!(row_ids(&host, "seat:").await, seats);
    assert!(row_ids(&host, "decided:").await.is_empty());
}

#[tokio::test]
async fn confirm_never_creates_a_seat_row_after_a_lapse() {
    let host = host();
    let slot_id = slot(&host, 2).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), NOW).await.unwrap();
    reservation::reserve_at(&host, reserve_input("agr-c", "c1", &slot_id), DUE + 1).await.unwrap();
    let seats = row_ids(&host, "seat:").await;
    let rsvs = row_ids(&host, "rsv:").await;
    for agreement in ["agr-a", "agr-b", "agr-c", "agr-none"] {
        // Each answer is checked in other tests; only the rows matter here.
        reservation::confirm_at(&host, key(agreement, "c1"), DUE + 2).await.ok();
    }
    assert_eq!(row_ids(&host, "seat:").await, seats);
    assert_eq!(row_ids(&host, "rsv:").await, rsvs);
}

#[tokio::test]
async fn a_release_after_a_lapse_changes_nothing() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), DUE + 1).await.unwrap();
    let out = reservation::release_at(&host, key("agr-a", "c1"), DUE + 2).await.unwrap();
    assert!(!out.released);
    // The seat still belongs to B.
    let third = reservation::reserve_at(&host, reserve_input("agr-c", "c1", &slot_id), DUE + 3);
    assert_eq!(code_of(&third.await.unwrap_err()), code::SLOT_TAKEN);
}

#[tokio::test]
async fn two_takers_of_one_lapsed_seat_give_it_to_one() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    // Both takers read the same seats, then race on the next round.
    let stale = reservation::read_seats(&host, &slot_id).await.unwrap();
    let slot_seats = reservation::SlotSeats { slot_id: &slot_id, capacity: 1, seats: &stale };
    let b = reservation::claim(&host, &key("agr-b", "c1"), 1, &slot_seats, DUE + 1).await.unwrap();
    let c = reservation::claim(&host, &key("agr-c", "c1"), 1, &slot_seats, DUE + 1).await.unwrap();
    assert!(matches!(b, reservation::Claim::Held(_)), "the first taker holds the seat: {b:?}");
    assert!(matches!(c, reservation::Claim::Full), "the second taker is refused: {c:?}");
    assert_eq!(row_ids(&host, "free:").await.len(), 1);
    assert_eq!(row_ids(&host, "seat:").await.len(), 2);
}

#[tokio::test]
async fn a_repeat_reserve_of_a_lapsed_key_takes_a_new_generation() {
    let host = host();
    let slot_id = slot(&host, 1).await;
    reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    reservation::reserve_at(&host, reserve_input("agr-b", "c1", &slot_id), DUE + 1).await.unwrap();
    // B releases; A reserves again after its own lapse and gets the seat.
    reservation::release_at(&host, key("agr-b", "c1"), DUE + 2).await.unwrap();
    let again = reservation::reserve_at(&host, reserve_input("agr-a", "c1", &slot_id), DUE + 3)
        .await
        .unwrap();
    assert_eq!(again.state, ReservationState::Held);
    assert_eq!(row_ids(&host, "rsv:").await.len(), 3);
}
