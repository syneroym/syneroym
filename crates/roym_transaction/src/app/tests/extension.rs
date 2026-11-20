//! How the transaction service treats a commitment's extension: the writer
//! refuses an unknown one, the counterparty files it, and a giver cannot
//! accept it.

use serde_json::{Value, json};
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    card::{self, CARD_CONTENT_TYPE},
    clock,
    deal::{Commitment, Details, ExtensionRef, SettlementRole, transition::CommitmentState},
    fulfilment::FULFILMENT_RECEIPT_VERSION,
    progress::AgreementState,
    record::{RECORD_FULFILMENT_RECEIPT, RECORD_QUOTE},
    testing::sd::{PlainFulfilment, conceal_fulfilment},
    transaction::{DealRole, QUOTE_VERSION},
};

use super::{
    accept::{author_service_commitment, quote_answering},
    disclosure::helpers::{build_and_sign_enquiry, sample_terms_input},
    fixture,
    settle::{
        incoming_card_row, make_currency_commitment, make_service_commitment,
        make_signed_fulfilment_envelope, sample_note, setup_agreed_deal,
    },
};
use crate::app::{
    AGREEMENTS, AgreementRef, AgreementRow, CARDS, CardRow, ENQUIRY_HISTORY,
    agreement_ops::{self, AgreementAcceptInput},
    fulfilment_ops::{self, FulfilmentSignInput},
    get_row,
    payment_ops::{self, PaymentAckInput},
    progress_ops, put_bytes,
    quote_ops::{self, QuoteSetInput},
    sync::{self, settle_receipts::file_fulfilment_card},
};

fn future_details() -> Details {
    Details {
        extension: "future".to_string(),
        schema: "future/thing@2".to_string(),
        data: json!({ "anything": [1, 2, 3] }),
        extra: vec![],
    }
}

fn one_shot_details(data: Value) -> Details {
    Details {
        extension: "one-shot".to_string(),
        schema: "one-shot/order@1".to_string(),
        data,
        extra: vec![],
    }
}

fn with_extension(mut c: Commitment, id: &str, details: Details) -> Commitment {
    c.extension = Some(ExtensionRef { id: id.to_string(), version: 1 });
    c.details = Some(details);
    c
}

/// A quote the author sets with `terms`, on a node that holds the enquiry.
async fn set_quote_error(commitment_input: impl FnOnce(&mut quote_ops::CommitmentInput)) -> Value {
    let author = fixture();
    let cp = fixture();
    let now = author.host.now();
    let (enq_rec, enq_env) = build_and_sign_enquiry(&cp.master, now);
    put_bytes(&author.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();

    let mut terms = sample_terms_input("Extension terms");
    commitment_input(&mut terms.commitments[0]);
    let err = quote_ops::set_at(
        &author.host,
        QuoteSetInput { enquiry_record_id: enq_rec, quote_id: None, expires_in_secs: 3600, terms },
        now,
    )
    .await
    .expect_err("quote.set must refuse these terms");
    err.error.as_ref().unwrap().data.clone().unwrap()
}

#[tokio::test]
async fn quote_set_refuses_an_unknown_extension() {
    let data = set_quote_error(|c| {
        c.extension = Some(ExtensionRef { id: "future".to_string(), version: 1 });
        c.details = Some(future_details());
    })
    .await;
    assert_eq!(data, json!({ "code": "extension-unknown" }));
}

#[tokio::test]
async fn quote_set_refuses_details_that_fail_the_schema() {
    let data = set_quote_error(|c| {
        c.extension = Some(ExtensionRef { id: "one-shot".to_string(), version: 1 });
        c.details = Some(one_shot_details(json!({ "colour": "red" })));
    })
    .await;
    assert_eq!(data, json!({ "code": "details-invalid" }));
}

/// The author signs `commitments` by hand, so a node that knows less than the
/// author can be shown a quote that the author's own node would refuse.
async fn counterparty_with_hand_signed_quote(
    commitments: impl FnOnce() -> Vec<Commitment>,
) -> (super::TestFixture, String, u64) {
    let cp = fixture();
    let author = Identity::generate().expect("generate author");
    let now = cp.host.now();
    let (enq_rec, enq_env) = build_and_sign_enquiry(&cp.master, now);
    put_bytes(&cp.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();

    let (quote, record_id, envelope) =
        quote_answering(&author, &cp.master_did, &enq_rec, commitments(), now + 3600, now);
    let body = card::card_body(RECORD_QUOTE, QUOTE_VERSION, &envelope, &quote.disclosures)
        .expect("quote card body");
    let msg = json!({
        "id": "msg_quote_1",
        "body": body,
        "content_type": CARD_CONTENT_TYPE,
        "sender_timestamp_ms": (now * 1000) as i64,
    });
    let filed = sync::file_incoming_card(&cp.host, &msg, "conv_1", now, &cp.master_did)
        .await
        .expect("file quote card");
    assert!(filed.filed && !filed.refused, "the quote card is filed");
    (cp, record_id, now)
}

#[tokio::test]
async fn counterparty_files_a_quote_with_an_unknown_extension() {
    let (cp, record_id, now) = counterparty_with_hand_signed_quote(|| {
        vec![with_extension(author_service_commitment("c1"), "future", future_details())]
    })
    .await;

    let card: CardRow = get_row(&cp.host, CARDS, "msg_quote_1").await.unwrap().unwrap();
    assert!(card.verified);
    assert_eq!(card.reason, None);
    let data = card.data.expect("plain terms are stored for the Hub");
    assert_eq!(data["terms"]["commitments"][0]["details"]["extension"], "future");

    // The author gives this commitment, so this node only receives it.
    let accepted = agreement_ops::accept_at(
        &cp.host,
        AgreementAcceptInput { quote_record_id: record_id, viewed_lang: None, viewed_via_mt: None },
        now,
    )
    .await
    .expect("a receiver may accept a part it cannot check");
    assert!(!accepted.record_id.is_empty());
}

#[tokio::test]
async fn accept_refuses_when_this_node_gives_an_unknown_extension() {
    let (cp, record_id, now) = counterparty_with_hand_signed_quote(|| {
        let mut given_here =
            with_extension(author_service_commitment("c1"), "future", future_details());
        given_here.giver = DealRole::Counterparty;
        vec![given_here]
    })
    .await;

    let err = agreement_ops::accept_at(
        &cp.host,
        AgreementAcceptInput {
            quote_record_id: record_id.clone(),
            viewed_lang: None,
            viewed_via_mt: None,
        },
        now,
    )
    .await
    .expect_err("a giver cannot accept a part it cannot run");
    assert_eq!(err.error.as_ref().unwrap().data, Some(json!({ "code": "extension-unknown" })));

    let row: Option<AgreementRow> = get_row(&cp.host, AGREEMENTS, &record_id).await.unwrap();
    assert!(row.is_none_or(|r| r.counterparty.is_none()), "nothing was signed");
}

#[tokio::test]
async fn one_shot_commitment_settles_like_today() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");
    let cp_did = substrate::derive_did_key(&counterparty.public_key());
    let agreed_at = clock::now_secs();
    let goods =
        with_extension(make_service_commitment("c1"), "one-shot", one_shot_details(json!({})));
    let payment = make_currency_commitment("c2", &fix.master_did);
    let (agreement, terms) =
        setup_agreed_deal(&fix, &counterparty, vec![goods, payment], agreed_at).await;
    terms.validate().expect("the one-shot deal is valid");

    payment_ops::acknowledge(
        &fix.host,
        PaymentAckInput {
            agreement: agreement.clone(),
            commitment_id: "c2".to_string(),
            method: Some("sepa-instant".to_string()),
            reference: Some("TX-1".to_string()),
            observed_at_secs: Some(agreed_at + 50),
            settlement_note: None,
            supersedes: None,
        },
    )
    .await
    .expect("payment acknowledgement");

    fulfilment_ops::sign_at(
        &fix.host,
        FulfilmentSignInput {
            agreement: agreement.clone(),
            commitment_id: "c1".to_string(),
            settlement_note: None,
        },
        agreed_at + 60,
    )
    .await
    .expect("giver fulfilment");

    let (receipt, disclosures) = conceal_fulfilment(
        PlainFulfilment {
            agreement: agreement.clone(),
            conversation: "conv_1".to_string(),
            commitment_id: "c1".to_string(),
            counterparty_did: cp_did.clone(),
            quote_author_did: fix.master_did.clone(),
            role: SettlementRole::Receiver,
            settlement_note: Some(sample_note("Received")),
        },
        1,
    );
    let envelope = make_signed_fulfilment_envelope(
        &counterparty,
        &cp_did,
        &agreement,
        &receipt,
        agreed_at + 70,
    );
    let row = incoming_card_row(
        "msg_fulfil_rc",
        "conv_1",
        RECORD_FULFILMENT_RECEIPT,
        FULFILMENT_RECEIPT_VERSION,
        &cp_did,
        agreed_at + 70,
    );
    let filed = file_fulfilment_card(
        &fix.host,
        row,
        &envelope,
        &disclosures,
        "conv_1",
        agreed_at + 70,
        &fix.master_did,
    )
    .await
    .expect("receiver fulfilment card");
    assert!(filed.filed && !filed.refused);

    let progress = progress_ops::progress_at(&fix.host, AgreementRef { agreement }, agreed_at + 80)
        .await
        .expect("progress");
    let snapshot = &progress.snapshots[0].snapshot;
    assert_eq!(snapshot.commitments[0].state, CommitmentState::Fulfilled);
    assert_eq!(snapshot.state, AgreementState::Closed);
}
