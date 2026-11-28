//! A receipt whose progress transition fails stays stored and is applied by the
//! next sync.

use super::*;

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
