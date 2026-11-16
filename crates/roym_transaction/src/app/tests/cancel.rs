use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    clock,
    deal::{Commitment, CommitmentKind, DealTerms, Details, ExtensionRef, LocalText, ResourceKind},
    lang::LangTag,
    listing::derive_slot_id,
    progress::{AgreementState, CancelReason},
    record::{Envelope, RECORD_AGREEMENT_RECEIPT, RecordDraft},
    transaction::{
        AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, DealRole, ReceiptHalf, TimeWindow,
        derive_quote_id,
    },
};
use z32;

use super::fixture;
use crate::app::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRow, LEDGER,
    decide::{SnapshotRow, snapshot_id},
    ensure_collections,
    fulfilment_ops::{self, FulfilmentSignInput},
    get_row,
    hold::{HoldLedgerRow, Reservations, SeatLedger, SeatLedgerRow, hold_id, seat_id},
    progress,
    progress_ops::{self, AgreementCancelInput},
    put_row,
};

fn make_service_commitment(id: &str) -> Commitment {
    Commitment {
        commitment_id: id.to_string(),
        giver: DealRole::QuoteAuthor,
        resource_kind: ResourceKind::GoodsOrService,
        kind: CommitmentKind::Fulfilment,
        extension: None,
        after: vec![],
        description: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "Piano lesson",
        ),
        declared_value_minor: 5000,
        details: None,
        window: None,
        location: None,
        payee: None,
        payment_methods: None,
        tax_minor: None,
        fees_minor: None,
        eep: None,
    }
}

fn make_slot_commitment(
    id: &str,
    listing_id: &str,
    start_secs: u64,
    end_secs: u64,
) -> (Commitment, String) {
    let slot_id = derive_slot_id(listing_id, start_secs, end_secs).expect("derive slot id");
    let commitment = Commitment {
        commitment_id: id.to_string(),
        giver: DealRole::QuoteAuthor,
        resource_kind: ResourceKind::ServiceInSlot,
        kind: CommitmentKind::Fulfilment,
        extension: Some(ExtensionRef { id: "scheduling".to_string(), version: 1 }),
        after: vec![],
        description: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "Piano lesson in slot",
        ),
        declared_value_minor: 5000,
        details: Some(Details {
            extension: "scheduling".to_string(),
            schema: "scheduling/slot@1".to_string(),
            data: json!({ "listing_id": listing_id, "slot_id": slot_id }),
            extra: vec![],
        }),
        window: Some(TimeWindow { earliest_secs: start_secs, latest_secs: end_secs }),
        location: None,
        payee: None,
        payment_methods: None,
        tax_minor: None,
        fees_minor: None,
        eep: None,
    };
    (commitment, slot_id)
}

fn make_receipt_half(
    signer: &Identity,
    role: DealRole,
    quote_record_id: &str,
    terms: &DealTerms,
    counterparty_did: &str,
    quote_author_did: &str,
    now: u64,
) -> ReceiptHalf {
    let signer_did = substrate::derive_did_key(&signer.public_key());
    let payload = AgreementReceiptPayload {
        quote_record_id: quote_record_id.to_string(),
        counterparty_did: counterparty_did.to_string(),
        quote_author_did: quote_author_did.to_string(),
        role,
        terms: terms.clone(),
        viewed_lang: LangTag::parse_canonical("en").expect("parse lang"),
        viewed_via_mt: false,
    };
    let draft = RecordDraft {
        version: AGREEMENT_RECEIPT_VERSION,
        record_type: RECORD_AGREEMENT_RECEIPT.to_string(),
        subject: quote_record_id.to_string(),
        payload: serde_json::to_value(&payload).expect("payload to value"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) =
        Envelope::unsigned(draft, signer_did.clone(), None, now).expect("draft envelope");
    let sig = signer.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("attach signature");
    let envelope_json = env.to_json().expect("envelope to json");
    let record_id = env.record_id().expect("record id");
    ReceiptHalf { record_id, envelope: envelope_json, issuer: signer_did, issued_at_secs: now }
}

async fn setup_agreed_deal(
    fix: &super::TestFixture,
    counterparty: &Identity,
    commitments: Vec<Commitment>,
    agreed_at: u64,
) -> (String, DealTerms) {
    ensure_collections(&fix.host).await.expect("ensure collections");

    let counterparty_did = substrate::derive_did_key(&counterparty.public_key());
    let quote_record_id = "rec_quote_cancel_test_1".to_string();
    let quote_id = derive_quote_id("conv_1", &fix.master_did, 1).expect("derive quote id");

    let terms = DealTerms {
        currency: "EUR".to_string(),
        commitments,
        home_exchange: None,
        listing_id: None,
        scope: LocalText::plain(LangTag::parse_canonical("en").expect("parse lang"), "Lesson deal"),
        cancellation_terms: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "24h notice",
        ),
        refund_terms: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "Full refund",
        ),
        dispute_path: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "Arbitration",
        ),
        quote_expires_at_secs: agreed_at + 3600,
    };

    let author_half = make_receipt_half(
        &fix.master,
        DealRole::QuoteAuthor,
        &quote_record_id,
        &terms,
        &counterparty_did,
        &fix.master_did,
        agreed_at,
    );
    let counterparty_half = make_receipt_half(
        counterparty,
        DealRole::Counterparty,
        &quote_record_id,
        &terms,
        &counterparty_did,
        &fix.master_did,
        agreed_at,
    );

    let snap1_payload = syneroym_roym_core::progress::open(
        quote_record_id.clone(),
        "conv_1".to_string(),
        counterparty_did.clone(),
        fix.master_did.clone(),
        &terms,
        None,
    );

    let (snap1_env, snap1_rec_id) =
        progress::sign(&fix.host, &snap1_payload).await.expect("sign snapshot 1");

    let snap_key = snapshot_id(&quote_record_id, 1);
    let snap_row = SnapshotRow {
        seq: 1,
        agreement: quote_record_id.clone(),
        conversation: "conv_1".to_string(),
        snapshot: snap1_payload.clone(),
        envelope: snap1_env,
        record_id: snap1_rec_id,
        message_id: None,
        stored_at_secs: agreed_at,
    };
    put_row(&fix.host, AGREEMENT_PROGRESS, &snap_key, &snap_row).await.expect("put snapshot row");

    let agr_row = AgreementRow {
        quote_record_id: quote_record_id.clone(),
        quote_id,
        conversation: "conv_1".to_string(),
        counterparty_did: counterparty_did.clone(),
        quote_author_did: fix.master_did.clone(),
        terms: terms.clone(),
        agreed_at_secs: Some(agreed_at),
        counterparty: Some(counterparty_half),
        quote_author: Some(author_half),
        state: Some(AgreementState::Agreed),
        state_reason: None,
        cancel_reason: None,
        seq: Some(1),
        commitments: snap1_payload.commitments,
        updated_at_secs: agreed_at,
    };
    put_row(&fix.host, AGREEMENTS, &quote_record_id, &agr_row).await.expect("put agreement row");

    (quote_record_id, terms)
}

#[tokio::test]
async fn cancel_is_refused_once_a_commitment_moved() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");
    let agreed_at = clock::now_secs();
    let (agreement, _terms) =
        setup_agreed_deal(&fix, &counterparty, vec![make_service_commitment("c1")], agreed_at)
            .await;

    // Giver signs fulfilment -> commitment moves to Claimed
    fulfilment_ops::sign_at(
        &fix.host,
        FulfilmentSignInput {
            agreement: agreement.clone(),
            commitment_id: "c1".to_string(),
            settlement_note: None,
        },
        agreed_at + 10,
    )
    .await
    .expect("sign fulfilment");

    // Attempting to cancel an agreement after commitment moved is refused
    let err = progress_ops::cancel_at(
        &fix.host,
        AgreementCancelInput {
            agreement: agreement.clone(),
            reason: CancelReason::ScheduleConflict,
        },
        agreed_at + 20,
    )
    .await
    .unwrap_err();

    assert!(err.error.unwrap().message.contains("commitment-moved"));
}

#[tokio::test]
async fn cancel_releases_writer_seat_holds() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");
    let agreed_at = clock::now_secs();

    let (comm, slot_id) = make_slot_commitment("c1", "lst_music", agreed_at + 100, agreed_at + 200);
    let (agreement, _terms) = setup_agreed_deal(&fix, &counterparty, vec![comm], agreed_at).await;

    // Reserve seat & confirm hold for the writer
    SeatLedger
        .reserve(&fix.host, &agreement, "c1", &slot_id, agreed_at)
        .await
        .expect("reserve seat");
    SeatLedger.confirm(&fix.host, &agreement, "c1").await.expect("confirm seat");

    let h_key = hold_id(&agreement, "c1");
    let s_key = seat_id(&slot_id, 1);

    // Verify hold and seat rows exist
    let h_before: Option<HoldLedgerRow> = get_row(&fix.host, LEDGER, &h_key).await.unwrap();
    let s_before: Option<SeatLedgerRow> = get_row(&fix.host, LEDGER, &s_key).await.unwrap();
    assert!(h_before.is_some());
    assert!(s_before.is_some());

    // Cancel agreement while commitments are still open
    let updated = progress_ops::cancel_at(
        &fix.host,
        AgreementCancelInput { agreement: agreement.clone(), reason: CancelReason::CannotDeliver },
        agreed_at + 10,
    )
    .await
    .expect("cancel agreement");

    assert_eq!(updated.state, Some(AgreementState::Cancelled));

    // Holds and seats must be released
    let h_after: Option<HoldLedgerRow> = get_row(&fix.host, LEDGER, &h_key).await.unwrap();
    let s_after: Option<SeatLedgerRow> = get_row(&fix.host, LEDGER, &s_key).await.unwrap();
    assert!(h_after.is_none(), "hold must be released on cancel");
    assert!(s_after.is_none(), "seat must be released on cancel");
}
