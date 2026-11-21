//! The writer files a signed commitment failure, and each giver releases the
//! reservations of its ended commitments.

use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    clock,
    commitment_failure::{COMMITMENT_FAILURE_VERSION, CommitmentFailurePayload, FailureReason},
    deal::{SettlementRole, transition::CommitmentState},
    paging,
    progress::{AgreementEvent, AgreementState},
    record::{Envelope, RECORD_COMMITMENT_FAILURE, RecordDraft},
    transaction::DealRole,
};

use super::{
    TestFixture, fixture,
    refuse::helpers::slot_commitment,
    settle::{
        incoming_card_row, make_currency_commitment, make_service_commitment, setup_agreed_deal,
    },
    step::StepDeal,
};
use crate::app::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRow, CARDS, CardRow, LEDGER, get_row,
    hooks::{self, ExtensionHooks, Reservations},
    ledger::{FailureRow, failure_id},
    progress_ops, put_row,
    sync::{FileCardResult, failure::file_commitment_failure_card},
};

/// A formed agreement; the node under test is the writer. `c1` is a slot that
/// the counterparty gives, `c2` is a service that the writer gives, and `c3`
/// is the counterparty's payment to the writer.
pub(super) async fn three_part_deal() -> StepDeal {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");
    let agreed_at = clock::now_secs();
    let (slot, _) = slot_commitment("c1", "listing_1", agreed_at + 1000, agreed_at + 4600);
    let service = make_service_commitment("c2");
    let payment = make_currency_commitment("c3", &fix.master_did);
    let (agreement, _) =
        setup_agreed_deal(&fix, &counterparty, vec![slot, service, payment], agreed_at).await;
    let cp_did = substrate::derive_did_key(&counterparty.public_key());
    StepDeal { fix, counterparty, cp_did, agreement, agreed_at }
}

/// A failure record of `commitment_id`, signed by `key` at `issued_at`.
pub(super) fn failure_envelope_at(
    deal: &StepDeal,
    commitment_id: &str,
    key: &Identity,
    issued_at: u64,
) -> String {
    let did = substrate::derive_did_key(&key.public_key());
    let payload = CommitmentFailurePayload {
        agreement: deal.agreement.clone(),
        conversation: "conv_1".to_string(),
        commitment_id: commitment_id.to_string(),
        reason: FailureReason::ReservationLost,
    };
    let draft = RecordDraft {
        version: COMMITMENT_FAILURE_VERSION,
        record_type: RECORD_COMMITMENT_FAILURE.to_string(),
        subject: deal.agreement.clone(),
        payload: serde_json::to_value(&payload).expect("payload to value"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, did, None, issued_at).unwrap();
    env.attach_signature(z32::encode(&key.sign(&bytes).to_bytes())).unwrap();
    env.to_json().unwrap()
}

fn failure_envelope(deal: &StepDeal, commitment_id: &str, key: &Identity) -> String {
    failure_envelope_at(deal, commitment_id, key, deal.agreed_at + 30)
}

pub(super) fn counterparty_failure(deal: &StepDeal, commitment_id: &str) -> String {
    failure_envelope(deal, commitment_id, &deal.counterparty)
}

pub(super) async fn file(
    deal: &StepDeal,
    msg: &str,
    envelope: &str,
    disclosures: &[String],
) -> FileCardResult {
    let row = incoming_card_row(
        msg,
        "conv_1",
        RECORD_COMMITMENT_FAILURE,
        COMMITMENT_FAILURE_VERSION,
        &deal.cp_did,
        deal.agreed_at + 30,
    );
    file_commitment_failure_card(
        &deal.fix.host,
        row,
        envelope,
        disclosures,
        "conv_1",
        deal.agreed_at + 40,
        &deal.fix.master_did,
    )
    .await
    .expect("filing does not fail")
}

pub(super) async fn card(fix: &TestFixture, msg: &str) -> CardRow {
    get_row(&fix.host, CARDS, msg).await.unwrap().expect("the card row")
}

pub(super) async fn agreement(deal: &StepDeal) -> AgreementRow {
    get_row(&deal.fix.host, AGREEMENTS, &deal.agreement).await.unwrap().expect("agreement row")
}

fn progress_of<'a>(row: &'a AgreementRow, id: &str) -> (&'a CommitmentState, Option<&'a str>) {
    let p = row.commitments.iter().find(|c| c.commitment_id == id).expect("commitment progress");
    (&p.state, p.reason.as_deref())
}

async fn snapshot_count(deal: &StepDeal) -> usize {
    let filter = serde_json::json!({ "agreement": deal.agreement }).to_string();
    paging::count(&deal.fix.host, AGREEMENT_PROGRESS, Some(filter)).await.unwrap()
}

async fn refused_reason(deal: &StepDeal, msg: &str, envelope: &str, d: &[String]) -> String {
    let res = file(deal, msg, envelope, d).await;
    assert!(res.refused, "the card is refused");
    card(&deal.fix, msg).await.reason.expect("a refusal reason")
}

#[tokio::test]
async fn the_writer_marks_the_commitment_failed_and_cancels_the_others() {
    let deal = three_part_deal().await;
    let res = file(&deal, "msg_f1", &counterparty_failure(&deal, "c1"), &[]).await;
    assert!(res.filed && !res.refused, "{res:?}");

    let row = agreement(&deal).await;
    assert_eq!(progress_of(&row, "c1"), (&CommitmentState::Failed, Some("reservation-lost")));
    assert_eq!(
        progress_of(&row, "c2"),
        (&CommitmentState::Cancelled, Some("other-commitment-failed"))
    );
    assert_eq!(
        progress_of(&row, "c3"),
        (&CommitmentState::Cancelled, Some("other-commitment-failed"))
    );
    assert_eq!(row.state, Some(AgreementState::Closed), "every commitment is final");
    assert_eq!(row.seq, Some(2));

    let stored: FailureRow = get_row(&deal.fix.host, LEDGER, &failure_id(&deal.agreement, "c1"))
        .await
        .unwrap()
        .expect("the filed record is stored");
    assert_eq!(stored.record_id, card(&deal.fix, "msg_f1").await.record_id.unwrap());
    assert!(card(&deal.fix, "msg_f1").await.verified);
}

#[tokio::test]
async fn a_commitment_with_a_half_keeps_its_path() {
    let deal = three_part_deal().await;
    let event = AgreementEvent::Half {
        commitment_id: "c3".to_string(),
        role: SettlementRole::Giver,
        record_id: "rec_half".to_string(),
    };
    progress_ops::transition_at(&deal.fix.host, &deal.agreement, event, deal.agreed_at + 20)
        .await
        .expect("the payer's half");

    file(&deal, "msg_f1", &counterparty_failure(&deal, "c1"), &[]).await;
    let row = agreement(&deal).await;
    assert_eq!(progress_of(&row, "c1").0, &CommitmentState::Failed);
    assert_eq!(progress_of(&row, "c3").0, &CommitmentState::Claimed);
    assert_eq!(row.state, Some(AgreementState::Agreed), "c3 is not final yet");
}

#[tokio::test]
async fn a_repeat_failure_card_makes_no_second_snapshot() {
    let deal = three_part_deal().await;
    let envelope = counterparty_failure(&deal, "c1");
    file(&deal, "msg_f1", &envelope, &[]).await;
    let before = snapshot_count(&deal).await;

    let res = file(&deal, "msg_f2", &envelope, &[]).await;
    assert!(res.filed && !res.refused);
    assert!(card(&deal.fix, "msg_f2").await.repeat);
    assert_eq!(snapshot_count(&deal).await, before);
}

#[tokio::test]
async fn a_second_failure_record_for_one_commitment_is_refused() {
    let deal = three_part_deal().await;
    file(&deal, "msg_f1", &counterparty_failure(&deal, "c1"), &[]).await;
    // Another record: signed at another time by the same giver.
    let other = failure_envelope_at(&deal, "c1", &deal.counterparty, deal.agreed_at + 31);
    let reason = refused_reason(&deal, "msg_f2", &other, &[]).await;
    assert!(reason.contains("second failure"), "{reason}");
}

#[tokio::test]
async fn the_writer_refuses_a_failure_from_the_wrong_signer() {
    let deal = three_part_deal().await;
    let stranger = Identity::generate().unwrap();
    let envelope = failure_envelope(&deal, "c1", &stranger);
    let reason = refused_reason(&deal, "msg_f1", &envelope, &[]).await;
    assert!(reason.contains("only the giver"), "{reason}");
    assert_eq!(agreement(&deal).await.state, Some(AgreementState::Agreed));
}

#[tokio::test]
async fn the_writer_refuses_a_failure_for_a_commitment_with_no_reserve_hook() {
    let deal = three_part_deal().await;
    let reason = refused_reason(&deal, "msg_f1", &counterparty_failure(&deal, "c3"), &[]).await;
    assert!(reason.contains("no reservation"), "{reason}");
}

#[tokio::test]
async fn the_writer_refuses_a_failure_for_a_commitment_the_writer_gives() {
    let deal = three_part_deal().await;
    let reason = refused_reason(&deal, "msg_f1", &counterparty_failure(&deal, "c2"), &[]).await;
    assert!(reason.contains("only the giver"), "{reason}");
}

#[tokio::test]
async fn the_writer_refuses_a_failure_with_a_settlement_half() {
    let deal = three_part_deal().await;
    let event = AgreementEvent::Half {
        commitment_id: "c1".to_string(),
        role: SettlementRole::Giver,
        record_id: "rec_half".to_string(),
    };
    progress_ops::transition_at(&deal.fix.host, &deal.agreement, event, deal.agreed_at + 20)
        .await
        .expect("the giver's half");
    let reason = refused_reason(&deal, "msg_f1", &counterparty_failure(&deal, "c1"), &[]).await;
    assert!(reason.contains("settlement half"), "{reason}");
}

#[tokio::test]
async fn the_writer_refuses_a_failure_before_the_agreement_formed() {
    let deal = three_part_deal().await;
    let mut row = agreement(&deal).await;
    row.state = None;
    put_row(&deal.fix.host, AGREEMENTS, &deal.agreement, &row).await.unwrap();
    let reason = refused_reason(&deal, "msg_f1", &counterparty_failure(&deal, "c1"), &[]).await;
    assert!(reason.contains("not agreed"), "{reason}");
}

#[tokio::test]
async fn the_writer_refuses_a_failure_with_disclosures() {
    let deal = three_part_deal().await;
    let reason =
        refused_reason(&deal, "msg_f1", &counterparty_failure(&deal, "c1"), &["x".to_string()])
            .await;
    assert!(reason.contains("disclosures not permitted"), "{reason}");
}

#[tokio::test]
async fn a_node_that_is_not_the_writer_changes_no_state() {
    let deal = three_part_deal().await;
    let row = incoming_card_row(
        "msg_f1",
        "conv_1",
        RECORD_COMMITMENT_FAILURE,
        COMMITMENT_FAILURE_VERSION,
        &deal.cp_did,
        deal.agreed_at + 30,
    );
    // Seen from the counterparty's node, the giver's own card is not acted on.
    let res = file_commitment_failure_card(
        &deal.fix.host,
        row,
        &counterparty_failure(&deal, "c1"),
        &[],
        "conv_1",
        deal.agreed_at + 40,
        &deal.cp_did,
    )
    .await
    .expect("filing");
    assert!(res.filed && !res.refused);
    assert_eq!(agreement(&deal).await.state, Some(AgreementState::Agreed));
    assert_eq!(snapshot_count(&deal).await, 1);
}

#[tokio::test]
async fn each_giver_releases_the_reservations_of_ended_commitments() {
    let deal = three_part_deal().await;
    // The writer also gives a slot, which it reserved and confirmed.
    let mut row = agreement(&deal).await;
    let writer_slot = row.terms.commitments[1].clone();
    let mut writer_slot = writer_slot;
    writer_slot.giver = DealRole::QuoteAuthor;
    writer_slot.extension = row.terms.commitments[0].extension.clone();
    writer_slot.details = row.terms.commitments[0].details.clone();
    row.terms.commitments[1] = writer_slot.clone();
    put_row(&deal.fix.host, AGREEMENTS, &deal.agreement, &row).await.unwrap();
    ExtensionHooks.reserve(&deal.fix.host, &deal.agreement, &writer_slot).await.unwrap();
    ExtensionHooks.confirm(&deal.fix.host, &deal.agreement, &writer_slot).await.unwrap();
    assert!(deal.fix.scheduling.is_confirmed(&deal.agreement, "c2"));

    file(&deal, "msg_f1", &counterparty_failure(&deal, "c1"), &[]).await;
    assert_eq!(
        progress_of(&agreement(&deal).await, "c2").0,
        &CommitmentState::Cancelled,
        "the writer's own commitment is cancelled by the failure"
    );
    assert!(deal.fix.scheduling.is_released(&deal.agreement, "c2"));
}

#[tokio::test]
async fn a_failed_release_of_an_ended_commitment_is_tried_again_by_the_next_sync() {
    let deal = three_part_deal().await;
    let mut row = agreement(&deal).await;
    let mut writer_slot = row.terms.commitments[1].clone();
    writer_slot.giver = DealRole::QuoteAuthor;
    writer_slot.extension = row.terms.commitments[0].extension.clone();
    writer_slot.details = row.terms.commitments[0].details.clone();
    row.terms.commitments[1] = writer_slot.clone();
    put_row(&deal.fix.host, AGREEMENTS, &deal.agreement, &row).await.unwrap();
    ExtensionHooks.reserve(&deal.fix.host, &deal.agreement, &writer_slot).await.unwrap();

    deal.fix.scheduling.fail_call("reservation.release", 0);
    file(&deal, "msg_f1", &counterparty_failure(&deal, "c1"), &[]).await;
    assert!(deal.fix.scheduling.failure_hit(), "the first release failed");
    assert!(deal.fix.scheduling.is_held(&deal.agreement, "c2"), "nothing freed the seat yet");

    // The next sync releases it, and records that it did.
    progress_ops::tick_conversation_at(&deal.fix.host, "conv_1", deal.agreed_at + 60)
        .await
        .expect("tick");
    assert!(deal.fix.scheduling.is_released(&deal.agreement, "c2"));
    let calls = deal.fix.scheduling.calls("reservation.release");

    progress_ops::tick_conversation_at(&deal.fix.host, "conv_1", deal.agreed_at + 61)
        .await
        .expect("tick again");
    assert_eq!(deal.fix.scheduling.calls("reservation.release"), calls, "no repeat call");
}

#[tokio::test]
async fn a_commitment_with_a_half_is_never_released() {
    let deal = three_part_deal().await;
    let row = agreement(&deal).await;
    let c1 = row.terms.commitments[0].clone();
    let progress = row.commitments.clone();
    let mut with_half = progress.clone();
    with_half[0].state = CommitmentState::Cancelled;
    with_half[0].halves = vec!["rec_half".to_string()];
    let ended = hooks::Ended { state: AgreementState::Agreed, commitments: &with_half };
    let mut as_giver = row.clone();
    as_giver.terms.commitments[0].giver = DealRole::QuoteAuthor;
    ExtensionHooks.reserve(&deal.fix.host, &deal.agreement, &c1).await.unwrap();
    hooks::release_ended(
        &deal.fix.host,
        &ExtensionHooks,
        &as_giver,
        DealRole::QuoteAuthor,
        ended,
        deal.agreed_at + 50,
    )
    .await;
    assert_eq!(deal.fix.scheduling.calls("reservation.release"), 0);
}
