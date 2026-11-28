//! A receipt whose progress transition fails stays stored and is applied by the
//! next sync.

use syneroym_roym_core::{
    payment::PAYMENT_ACKNOWLEDGEMENT_VERSION,
    record::RECORD_PAYMENT_ACKNOWLEDGEMENT,
    testing::sd::{PlainPaymentAck, conceal_payment_ack},
};

use super::*;
use crate::app::sync::settle_receipts::file_payment_ack_card;

#[tokio::test]
async fn a_progress_failure_defers_the_receipt_and_the_next_filing_applies_it() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");
    let agreed_at = clock::now_secs();
    let (agreement, _terms) =
        setup_agreed_deal(&fix, &counterparty, vec![make_service_commitment("c1")], agreed_at)
            .await;

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
    .expect("author sign fulfilment");

    let cp_did = substrate::derive_did_key(&counterparty.public_key());
    let plain = PlainFulfilment {
        agreement: agreement.clone(),
        conversation: "conv_1".to_string(),
        commitment_id: "c1".to_string(),
        counterparty_did: cp_did.clone(),
        quote_author_did: fix.master_did.clone(),
        role: SettlementRole::Receiver,
        settlement_note: None,
    };
    let (payload, disclosures) = conceal_fulfilment(plain, 1);
    let envelope_json = make_signed_fulfilment_envelope(
        &counterparty,
        &cp_did,
        &agreement,
        &payload,
        agreed_at + 20,
    );

    fix.host.fail_write(AGREEMENT_PROGRESS, format!("snapshot:{agreement}:"));
    let first = file_fulfilment_card(
        &fix.host,
        incoming_card_row(
            "msg_fulfil_transition",
            "conv_1",
            RECORD_FULFILMENT_RECEIPT,
            FULFILMENT_RECEIPT_VERSION,
            &cp_did,
            agreed_at + 20,
        ),
        &envelope_json,
        &disclosures,
        "conv_1",
        agreed_at + 20,
        &fix.master_did,
    )
    .await
    .expect("first filing");
    assert!(!first.filed);
    assert!(first.deferred);

    let f_id = fulfilment_id(&agreement, "c1");
    let stored: Option<FulfilmentsRow> = get_row(&fix.host, FULFILMENTS, &f_id).await.unwrap();
    assert!(stored.is_some(), "the received half stays stored");
    let prog = progress_ops::progress_at(
        &fix.host,
        AgreementRef { agreement: agreement.clone() },
        agreed_at + 30,
    )
    .await
    .expect("get progress");
    assert_eq!(prog.snapshots[0].snapshot.commitments[0].state, CommitmentState::Claimed);

    let second = file_fulfilment_card(
        &fix.host,
        incoming_card_row(
            "msg_fulfil_transition",
            "conv_1",
            RECORD_FULFILMENT_RECEIPT,
            FULFILMENT_RECEIPT_VERSION,
            &cp_did,
            agreed_at + 20,
        ),
        &envelope_json,
        &disclosures,
        "conv_1",
        agreed_at + 21,
        &fix.master_did,
    )
    .await
    .expect("second filing");
    assert!(second.filed);
    assert!(!second.deferred);

    let prog = progress_ops::progress_at(&fix.host, AgreementRef { agreement }, agreed_at + 31)
        .await
        .expect("get progress");
    assert_eq!(prog.snapshots[0].snapshot.commitments[0].state, CommitmentState::Fulfilled);
}

#[tokio::test]
async fn a_payment_ack_deferral_also_survives_a_retry() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");
    let agreed_at = clock::now_secs();
    let (agreement, _terms) = setup_agreed_deal(
        &fix,
        &counterparty,
        vec![make_currency_commitment("c2", &fix.master_did)],
        agreed_at,
    )
    .await;

    let cp_did = substrate::derive_did_key(&counterparty.public_key());
    let plain = PlainPaymentAck {
        agreement: agreement.clone(),
        conversation: "conv_1".to_string(),
        commitment_id: "c2".to_string(),
        quote_author_did: fix.master_did.clone(),
        counterparty_did: cp_did.clone(),
        role: SettlementRole::Giver,
        observed_at_secs: agreed_at + 10,
        method: None,
        reference: None,
        settlement_note: None,
    };
    let (payload, disclosures) = conceal_payment_ack(plain, 1);
    let draft = RecordDraft {
        version: PAYMENT_ACKNOWLEDGEMENT_VERSION,
        record_type: RECORD_PAYMENT_ACKNOWLEDGEMENT.to_string(),
        subject: agreement.clone(),
        payload: serde_json::to_value(&payload).expect("ack payload"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) =
        Envelope::unsigned(draft, cp_did.clone(), None, agreed_at + 10).expect("ack draft");
    let sig = counterparty.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("attach ack signature");
    let envelope_json = env.to_json().expect("ack envelope");

    fix.host.fail_write(AGREEMENT_PROGRESS, format!("snapshot:{agreement}:"));
    let first = file_payment_ack_card(
        &fix.host,
        incoming_card_row(
            "msg_ack_retry",
            "conv_1",
            RECORD_PAYMENT_ACKNOWLEDGEMENT,
            PAYMENT_ACKNOWLEDGEMENT_VERSION,
            &cp_did,
            agreed_at + 10,
        ),
        &envelope_json,
        &disclosures,
        "conv_1",
        agreed_at + 10,
        &fix.master_did,
    )
    .await
    .expect("first ack filing");
    assert!(!first.filed);
    assert!(first.deferred);
    assert!(!first.refused);

    let second = file_payment_ack_card(
        &fix.host,
        incoming_card_row(
            "msg_ack_retry",
            "conv_1",
            RECORD_PAYMENT_ACKNOWLEDGEMENT,
            PAYMENT_ACKNOWLEDGEMENT_VERSION,
            &cp_did,
            agreed_at + 10,
        ),
        &envelope_json,
        &disclosures,
        "conv_1",
        agreed_at + 11,
        &fix.master_did,
    )
    .await
    .expect("second ack filing");
    assert!(second.filed);
    assert!(!second.deferred);

    let prog = progress_ops::progress_at(&fix.host, AgreementRef { agreement }, agreed_at + 30)
        .await
        .expect("get progress");
    assert_eq!(prog.snapshots[0].snapshot.commitments[0].state, CommitmentState::Claimed);
}

#[tokio::test]
async fn a_progress_failure_after_seven_days_is_deferred_not_refused() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");
    let agreed_at = clock::now_secs();
    let (agreement, _terms) =
        setup_agreed_deal(&fix, &counterparty, vec![make_service_commitment("c1")], agreed_at)
            .await;

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
    .expect("author sign fulfilment");

    let cp_did = substrate::derive_did_key(&counterparty.public_key());
    let plain = PlainFulfilment {
        agreement: agreement.clone(),
        conversation: "conv_1".to_string(),
        commitment_id: "c1".to_string(),
        counterparty_did: cp_did.clone(),
        quote_author_did: fix.master_did.clone(),
        role: SettlementRole::Receiver,
        settlement_note: None,
    };
    let (payload, disclosures) = conceal_fulfilment(plain, 1);
    let envelope_json = make_signed_fulfilment_envelope(
        &counterparty,
        &cp_did,
        &agreement,
        &payload,
        agreed_at + 20,
    );
    let late_now = agreed_at + 20 + 8 * 24 * 60 * 60;

    fix.host.fail_write(AGREEMENT_PROGRESS, format!("snapshot:{agreement}:"));
    let first = file_fulfilment_card(
        &fix.host,
        incoming_card_row(
            "msg_fulfil_late",
            "conv_1",
            RECORD_FULFILMENT_RECEIPT,
            FULFILMENT_RECEIPT_VERSION,
            &cp_did,
            agreed_at + 20,
        ),
        &envelope_json,
        &disclosures,
        "conv_1",
        late_now,
        &fix.master_did,
    )
    .await
    .expect("late filing");
    assert!(!first.filed);
    assert!(first.deferred);
    assert!(!first.refused);
}

#[tokio::test]
async fn a_progress_read_failure_defers_the_receipt_and_the_next_filing_applies_it() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");
    let agreed_at = clock::now_secs();
    let (agreement, _terms) =
        setup_agreed_deal(&fix, &counterparty, vec![make_service_commitment("c1")], agreed_at)
            .await;

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
    .expect("author sign fulfilment");

    let cp_did = substrate::derive_did_key(&counterparty.public_key());
    let plain = PlainFulfilment {
        agreement: agreement.clone(),
        conversation: "conv_1".to_string(),
        commitment_id: "c1".to_string(),
        counterparty_did: cp_did.clone(),
        quote_author_did: fix.master_did.clone(),
        role: SettlementRole::Receiver,
        settlement_note: None,
    };
    let (payload, disclosures) = conceal_fulfilment(plain, 1);
    let envelope_json = make_signed_fulfilment_envelope(
        &counterparty,
        &cp_did,
        &agreement,
        &payload,
        agreed_at + 20,
    );

    fix.host.fail_read(AGREEMENT_PROGRESS, "");
    let first = file_fulfilment_card(
        &fix.host,
        incoming_card_row(
            "msg_fulfil_read",
            "conv_1",
            RECORD_FULFILMENT_RECEIPT,
            FULFILMENT_RECEIPT_VERSION,
            &cp_did,
            agreed_at + 20,
        ),
        &envelope_json,
        &disclosures,
        "conv_1",
        agreed_at + 20,
        &fix.master_did,
    )
    .await
    .expect("first filing");
    assert!(!first.filed);
    assert!(first.deferred);
    assert!(!first.refused);

    let second = file_fulfilment_card(
        &fix.host,
        incoming_card_row(
            "msg_fulfil_read",
            "conv_1",
            RECORD_FULFILMENT_RECEIPT,
            FULFILMENT_RECEIPT_VERSION,
            &cp_did,
            agreed_at + 20,
        ),
        &envelope_json,
        &disclosures,
        "conv_1",
        agreed_at + 21,
        &fix.master_did,
    )
    .await
    .expect("second filing");
    assert!(second.filed);

    let prog = progress_ops::progress_at(&fix.host, AgreementRef { agreement }, agreed_at + 30)
        .await
        .expect("get progress");
    assert_eq!(prog.snapshots[0].snapshot.commitments[0].state, CommitmentState::Fulfilled);
}
