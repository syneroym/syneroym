//! An import takes again the reservations of the agreements it restores.

use std::sync::Arc;

use syneroym_roym_core::{
    backup::ImportInput, listing::derive_slot_id, method::NoParams, progress::CancelReason,
    transaction::DealRole,
};

use super::{
    TestFixture, fixture_with_master,
    step::{StepDeal, slot_deal},
};
use crate::app::{
    backup,
    progress_ops::{self, AgreementCancelInput},
};

/// The slot of `slot_deal`'s booking `c1`.
fn slot_of(deal: &StepDeal) -> String {
    derive_slot_id("listing_1", deal.agreed_at + 1000, deal.agreed_at + 4600).expect("slot id")
}

async fn restore_into_new_node(
    deal: &StepDeal,
    prepare: impl FnOnce(&TestFixture),
) -> (TestFixture, backup::TransactionImportOutput) {
    let bundle = backup::export(&deal.fix.host, NoParams {}).await.expect("export");
    let target = fixture_with_master(Arc::clone(&deal.fix.master));
    prepare(&target);
    let out = backup::import(&target.host, ImportInput { bundle }).await.expect("import");
    (target, out)
}

#[tokio::test]
async fn import_reserves_again_the_seat_of_an_agreed_deal() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let (target, out) = restore_into_new_node(&deal, |_| {}).await;

    assert_eq!(out.reservations_restored, 1);
    assert!(out.reservations_failed.is_empty(), "{:?}", out.reservations_failed);
    assert!(target.scheduling.is_confirmed(&deal.agreement, "c1"), "reserved and confirmed");
    assert_eq!(target.scheduling.held_seats(&slot_of(&deal)), 1);
    assert_eq!(target.scheduling.calls("reservation.reserve"), 1);
    assert_eq!(target.scheduling.calls("reservation.confirm"), 1);
}

#[tokio::test]
async fn import_finds_a_restored_reservation() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    let slot_id = slot_of(&deal);
    let (target, out) =
        restore_into_new_node(&deal, |t| t.scheduling.take_seat(&slot_id, &deal.agreement)).await;

    assert_eq!(out.reservations_restored, 1);
    assert!(out.reservations_failed.is_empty());
    assert_eq!(target.scheduling.held_seats(&slot_id), 1, "the restored seat is not held twice");
}

#[tokio::test]
async fn import_before_scheduling_lists_the_failed_seats() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    // The scheduling bundle is not imported yet, so its slot is not there.
    let slot_id = slot_of(&deal);
    let (target, out) = restore_into_new_node(&deal, |t| t.scheduling.remove_slot(&slot_id)).await;

    assert!(out.imported, "the import itself succeeds");
    assert_eq!(out.reservations_restored, 0);
    assert_eq!(out.reservations_failed.len(), 1);
    let failed = &out.reservations_failed[0];
    assert_eq!(failed.agreement, deal.agreement);
    assert_eq!(failed.commitment_id, "c1");
    assert_eq!(failed.reason, "slot-unavailable");
    assert!(!target.scheduling.is_held(&deal.agreement, "c1"));
}

#[tokio::test]
async fn an_ended_agreement_is_not_reserved() {
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    progress_ops::cancel_at(
        &deal.fix.host,
        AgreementCancelInput {
            agreement: deal.agreement.clone(),
            reason: CancelReason::CannotDeliver,
        },
        deal.agreed_at + 10,
    )
    .await
    .expect("cancel");
    let (target, out) = restore_into_new_node(&deal, |_| {}).await;

    assert_eq!(out.reservations_restored, 0);
    assert!(out.reservations_failed.is_empty());
    assert!(target.scheduling.call_log().is_empty(), "no hook call for an ended agreement");
}

#[tokio::test]
async fn a_commitment_that_the_other_side_gives_is_not_reserved() {
    let deal = slot_deal(DealRole::Counterparty).await;
    let (target, out) = restore_into_new_node(&deal, |_| {}).await;

    assert_eq!(out.reservations_restored, 0);
    assert!(target.scheduling.call_log().is_empty());
}
