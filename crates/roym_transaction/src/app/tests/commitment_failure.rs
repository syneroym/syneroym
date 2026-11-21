//! The counterparty's confirm of its own reservations, and the signed
//! failure that a refused confirm leads to.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use syneroym_app_host::types::proxy::ProxyError;
use syneroym_roym_core::{
    commitment_failure::{FailureReason, verify_commitment_failure},
    deal::{Commitment, transition::CommitmentState},
    progress::AgreementState,
    signing::{CERTIFICATE_ID, CERTIFICATES, StoredCertificate},
    transaction::DealRole,
};

use super::{
    commitment_failure_writer::agreement,
    step::{StepDeal, slot_deal},
};
use crate::app::{
    AGREEMENTS, AgreementRow, LEDGER, get_row,
    hooks::{ExtensionHooks, Reservations},
    ledger::{FailureRow, LedgerKind, LedgerRow, confirmed_id, failure_id},
    put_row,
    sync::confirm::confirm_own_reservations,
};

/// A formed agreement whose slot `c1` is given by the node under test, which is
/// the counterparty here. The writer's countersigned half is held, and the
/// node has reserved its own seat.
async fn counterparty_deal() -> StepDeal {
    let deal = slot_deal(DealRole::Counterparty).await;
    let mut row = agreement(&deal).await;
    row.quote_author_did = deal.cp_did.clone();
    row.counterparty_did = deal.fix.master_did.clone();
    put_row(&deal.fix.host, AGREEMENTS, &deal.agreement, &row).await.unwrap();
    let held = ExtensionHooks.reserve(&deal.fix.host, &deal.agreement, &slot(&row)).await;
    held.expect("the node reserves its own seat");
    deal
}

fn slot(row: &AgreementRow) -> Commitment {
    row.terms.commitments.iter().find(|c| c.commitment_id == "c1").cloned().expect("c1")
}

async fn edit_agreement(deal: &StepDeal, edit: impl FnOnce(&mut AgreementRow)) {
    let mut row = agreement(deal).await;
    edit(&mut row);
    put_row(&deal.fix.host, AGREEMENTS, &deal.agreement, &row).await.unwrap();
}

async fn pass(deal: &StepDeal, now: u64) -> u32 {
    confirm_own_reservations(&deal.fix.host, &ExtensionHooks, "conv_1", &deal.fix.master_did, now)
        .await
        .expect("the pass does not fail")
}

async fn failure_row(deal: &StepDeal) -> Option<FailureRow> {
    get_row(&deal.fix.host, LEDGER, &failure_id(&deal.agreement, "c1")).await.unwrap()
}

async fn confirmed_row(deal: &StepDeal) -> Option<LedgerRow> {
    get_row(&deal.fix.host, LEDGER, &confirmed_id(&deal.agreement, "c1")).await.unwrap()
}

/// The cards of type `commitment-failure` that the node sent.
fn sent_failures(deal: &StepDeal) -> Vec<Value> {
    deal.fix
        .host
        .sent_cards()
        .iter()
        .filter_map(|body| serde_json::from_str::<Value>(body).ok())
        .filter(|card| card["type"] == "commitment-failure")
        .collect()
}

#[tokio::test]
async fn a_confirmed_commitment_is_not_confirmed_again() {
    let deal = counterparty_deal().await;
    assert_eq!(pass(&deal, deal.agreed_at + 5).await, 0);
    assert!(deal.fix.scheduling.is_confirmed(&deal.agreement, "c1"));
    assert_eq!(confirmed_row(&deal).await.expect("the marker").kind, LedgerKind::Confirmed);
    assert_eq!(pass(&deal, deal.agreed_at + 6).await, 0);
    assert_eq!(deal.fix.scheduling.calls("reservation.confirm"), 1);
    assert!(sent_failures(&deal).is_empty());
}

#[tokio::test]
async fn a_refused_confirm_signs_one_failure_and_sends_it() {
    let deal = counterparty_deal().await;
    deal.fix.scheduling.refuse_confirms("reservation-lost");
    assert_eq!(pass(&deal, deal.agreed_at + 5).await, 0);

    let stored = failure_row(&deal).await.expect("the failure row");
    assert!(stored.message_id.is_some(), "the send is recorded");
    let verdict = verify_commitment_failure(&stored.envelope, deal.agreed_at + 5);
    assert!(verdict.verified, "{:?}", verdict.reason);
    assert_eq!(verdict.issuer.as_deref(), Some(deal.fix.master_did.as_str()));
    let payload = verdict.payload.expect("payload");
    assert_eq!(payload.agreement, deal.agreement);
    assert_eq!(payload.commitment_id, "c1");
    assert_eq!(payload.reason, FailureReason::ReservationLost);
    assert_eq!(sent_failures(&deal).len(), 1);

    // A second sync makes no new record and sends nothing.
    assert_eq!(pass(&deal, deal.agreed_at + 6).await, 0);
    assert_eq!(sent_failures(&deal).len(), 1);
    assert_eq!(deal.fix.scheduling.calls("reservation.confirm"), 1);
    assert_eq!(failure_row(&deal).await.unwrap().record_id, stored.record_id);
    assert!(confirmed_row(&deal).await.is_none());
}

#[tokio::test]
async fn every_refusal_code_ends_as_a_lost_reservation() {
    let deal = counterparty_deal().await;
    deal.fix.scheduling.refuse_confirms("reservation-unknown");
    pass(&deal, deal.agreed_at + 5).await;
    let stored = failure_row(&deal).await.expect("the failure row");
    let payload = verify_commitment_failure(&stored.envelope, deal.agreed_at + 5).payload;
    assert_eq!(payload.expect("payload").reason, FailureReason::ReservationLost);
}

/// A conversation that fails its sends while `down` is set, and keeps every
/// card that it did accept.
fn flaky_conversation(deal: &StepDeal) -> (Arc<Mutex<bool>>, Arc<Mutex<Vec<String>>>) {
    let down = Arc::new(Mutex::new(true));
    let kept = Arc::new(Mutex::new(Vec::new()));
    let (is_down, store) = (Arc::clone(&down), Arc::clone(&kept));
    deal.fix.host.on_call("conversation", "conversation.send", move |params| {
        if *is_down.lock().unwrap() {
            return Err(ProxyError::Transport("conversation is down".to_string()));
        }
        let body = serde_json::from_str::<Value>(&params).unwrap()["body"].to_string();
        store.lock().unwrap().push(body);
        Ok(json!({ "message_id": "msg_ok", "state": "delivered", "sender_timestamp_ms": 1 })
            .to_string())
    });
    (down, kept)
}

#[tokio::test]
async fn a_send_error_resends_the_stored_failure() {
    let deal = counterparty_deal().await;
    let (down, kept) = flaky_conversation(&deal);
    deal.fix.scheduling.refuse_confirms("reservation-lost");

    assert_eq!(pass(&deal, deal.agreed_at + 5).await, 1, "the failure is reported as unsent");
    let first = failure_row(&deal).await.expect("the row is kept");
    assert!(first.message_id.is_none());
    assert!(kept.lock().unwrap().is_empty());

    *down.lock().unwrap() = false;
    assert_eq!(pass(&deal, deal.agreed_at + 6).await, 0);
    let second = failure_row(&deal).await.unwrap();
    assert_eq!(second.record_id, first.record_id, "the same record is sent again");
    assert_eq!(second.message_id.as_deref(), Some("msg_ok"));
    assert_eq!(kept.lock().unwrap().len(), 1);
    assert_eq!(deal.fix.scheduling.calls("reservation.confirm"), 1, "no second confirm");
}

#[tokio::test]
async fn a_transport_error_on_confirm_retries_on_the_next_sync() {
    let deal = counterparty_deal().await;
    deal.fix.scheduling.fail_call("reservation.confirm", 0);
    assert_eq!(pass(&deal, deal.agreed_at + 5).await, 0);
    assert!(failure_row(&deal).await.is_none(), "a fault is not a refusal");
    assert!(confirmed_row(&deal).await.is_none());

    assert_eq!(pass(&deal, deal.agreed_at + 6).await, 0);
    assert!(confirmed_row(&deal).await.is_some());
    assert!(sent_failures(&deal).is_empty());
}

#[tokio::test]
async fn a_failure_that_cannot_be_signed_is_retried_and_reported() {
    let deal = counterparty_deal().await;
    let host = &deal.fix.host;
    let good: StoredCertificate =
        get_row(host, CERTIFICATES, CERTIFICATE_ID).await.unwrap().unwrap();
    host.set_storage_row(
        CERTIFICATES,
        CERTIFICATE_ID,
        &StoredCertificate { expires_at_secs: 1, ..good.clone() },
    );
    deal.fix.scheduling.refuse_confirms("reservation-lost");

    assert_eq!(pass(&deal, deal.agreed_at + 5).await, 1);
    assert!(failure_row(&deal).await.is_none(), "no row, so no unsigned record can be sent");
    assert!(sent_failures(&deal).is_empty());

    host.set_storage_row(CERTIFICATES, CERTIFICATE_ID, &good);
    assert_eq!(pass(&deal, deal.agreed_at + 6).await, 0);
    assert_eq!(sent_failures(&deal).len(), 1);
    assert!(failure_row(&deal).await.is_some());
}

#[tokio::test]
async fn a_late_half_after_the_expiry_release_gives_a_commitment_failure() {
    let deal = counterparty_deal().await;
    // The quote expired with no countersigned half, and the node released
    // its seat. Then the writer's half, signed before the expiry, is filed.
    let row = agreement(&deal).await;
    ExtensionHooks.release(&deal.fix.host, &deal.agreement, &slot(&row)).await.unwrap();
    assert!(deal.fix.scheduling.is_released(&deal.agreement, "c1"));

    assert_eq!(pass(&deal, row.terms.quote_expires_at_secs + 60).await, 0);
    assert_eq!(sent_failures(&deal).len(), 1, "a confirm never takes the seat again");
    assert!(deal.fix.scheduling.is_released(&deal.agreement, "c1"));
    assert!(failure_row(&deal).await.is_some());
}

#[tokio::test]
async fn an_ended_commitment_is_never_confirmed() {
    let deal = counterparty_deal().await;
    edit_agreement(&deal, |row| {
        for p in &mut row.commitments {
            p.state = CommitmentState::Cancelled;
            p.reason = Some("other-commitment-failed".to_string());
        }
    })
    .await;
    assert_eq!(pass(&deal, deal.agreed_at + 5).await, 0);
    assert_eq!(deal.fix.scheduling.calls("reservation.confirm"), 0);
    assert!(sent_failures(&deal).is_empty());
}

#[tokio::test]
async fn nothing_is_confirmed_before_the_writer_countersigned() {
    let deal = counterparty_deal().await;
    edit_agreement(&deal, |row| row.quote_author = None).await;
    assert_eq!(pass(&deal, deal.agreed_at + 5).await, 0);
    assert_eq!(deal.fix.scheduling.calls("reservation.confirm"), 0);
}

#[tokio::test]
async fn nothing_is_confirmed_once_the_agreement_has_ended() {
    let deal = counterparty_deal().await;
    edit_agreement(&deal, |row| {
        row.state = Some(AgreementState::Closed);
    })
    .await;
    assert_eq!(pass(&deal, deal.agreed_at + 5).await, 0);
    assert_eq!(deal.fix.scheduling.calls("reservation.confirm"), 0);
}

#[tokio::test]
async fn the_writer_confirms_nothing_here() {
    // The node under test is the quote author: its own seat is confirmed at
    // the decision, never by this pass.
    let deal = slot_deal(DealRole::QuoteAuthor).await;
    assert_eq!(pass(&deal, deal.agreed_at + 5).await, 0);
    assert_eq!(deal.fix.scheduling.calls("reservation.confirm"), 0);
}

#[tokio::test]
async fn a_commitment_of_the_other_party_is_not_confirmed() {
    let deal = slot_deal(DealRole::Counterparty).await;
    // The node is the writer and the slot is given by the other side.
    assert_eq!(pass(&deal, deal.agreed_at + 5).await, 0);
    assert_eq!(deal.fix.scheduling.calls("reservation.confirm"), 0);
}

#[tokio::test]
async fn sync_runs_the_confirm_pass_and_reports_unsent_failures() {
    use crate::app::sync::{SyncInput, sync_at};

    let deal = counterparty_deal().await;
    let (_down, kept) = flaky_conversation(&deal);
    deal.fix.scheduling.refuse_confirms("reservation-lost");
    let input = || SyncInput { conversation: "conv_1".to_string(), full: false };

    let out = sync_at(&deal.fix.host, input(), deal.agreed_at + 5).await.expect("sync");
    assert_eq!(out.unsent_failures, 1);
    assert!(failure_row(&deal).await.is_some());
    assert!(kept.lock().unwrap().is_empty());
}

mod snapshot {
    use syneroym_identity::{Identity, substrate};
    use syneroym_roym_core::{
        deal::{DealTerms, transition::CommitmentState},
        progress::{
            self, AGREEMENT_PROGRESS_VERSION, AgreementProgressPayload, AgreementState,
            RECORD_AGREEMENT_PROGRESS,
        },
        record::{Envelope, RECORD_QUOTE, RecordDraft},
        transaction::verify_quote,
    };

    use super::super::{
        TestFixture,
        accept::{build_and_sign_quote, slot_commitment},
        fixture,
        settle::incoming_card_row,
    };
    use crate::app::{
        agreement_ops::{self, AgreementAcceptInput},
        disclosures,
        quote_ops::store_received_quote,
        sync::receipts::file_progress_card,
    };

    fn signed(author: &Identity, payload: &AgreementProgressPayload, now: u64) -> String {
        let did = substrate::derive_did_key(&author.public_key());
        let draft = RecordDraft {
            version: AGREEMENT_PROGRESS_VERSION,
            record_type: RECORD_AGREEMENT_PROGRESS.to_string(),
            subject: payload.agreement.clone(),
            payload: serde_json::to_value(payload).expect("serialize payload"),
            expires_at_secs: None,
            supersedes: None,
        };
        let (mut env, bytes) = Envelope::unsigned(draft, did, None, now).expect("draft progress");
        env.attach_signature(z32::encode(&author.sign(&bytes).to_bytes())).unwrap();
        env.to_json().unwrap()
    }

    async fn file(
        f: &TestFixture,
        author: &Identity,
        payload: &AgreementProgressPayload,
        msg: &str,
        now: u64,
    ) {
        let did = substrate::derive_did_key(&author.public_key());
        let row = incoming_card_row(msg, "conv_1", RECORD_AGREEMENT_PROGRESS, 1, &did, now);
        let res = file_progress_card(
            &f.host,
            row,
            &signed(author, payload, now),
            &[],
            "conv_1",
            now,
            &f.master_did,
        )
        .await
        .expect("file progress card");
        assert!(res.filed && !res.refused, "{res:?}");
    }

    #[tokio::test]
    async fn the_counterparty_releases_its_failed_commitment_when_it_files_the_snapshot() {
        let f = fixture();
        let now = f.host.now();
        let author = Identity::generate().expect("generate author");
        let author_did = substrate::derive_did_key(&author.public_key());
        let (c1, _) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);
        let (quote, rec_id, env) =
            build_and_sign_quote(&author, &f.master_did, vec![c1], now + 3600, now);
        let verdict = verify_quote(&env, now);
        store_received_quote(&f.host, &env, &verdict, &quote.signed, now, &f.master_did)
            .await
            .expect("store quote");
        disclosures::store(&f.host, &rec_id, RECORD_QUOTE, &quote.disclosures, now)
            .await
            .expect("store disclosures");
        let accept = AgreementAcceptInput {
            quote_record_id: rec_id.clone(),
            viewed_lang: None,
            viewed_via_mt: None,
        };
        agreement_ops::accept_at(&f.host, accept, now).await.expect("accept");
        assert!(f.scheduling.is_held(&rec_id, "c1"));

        let terms: &DealTerms = &quote.terms;
        let mut snap = progress::open(
            rec_id.clone(),
            "conv_1".to_string(),
            f.master_did.clone(),
            author_did,
            terms,
            None,
        );
        file(&f, &author, &snap, "msg_p1", now + 10).await;
        assert!(f.scheduling.is_held(&rec_id, "c1"), "an agreed snapshot releases nothing");

        snap.seq = 2;
        snap.commitments[0].state = CommitmentState::Failed;
        snap.commitments[0].reason = Some("reservation-lost".to_string());
        snap.state = AgreementState::Closed;
        file(&f, &author, &snap, "msg_p2", now + 20).await;
        assert!(f.scheduling.is_released(&rec_id, "c1"), "the failed commitment is released");
    }
}
