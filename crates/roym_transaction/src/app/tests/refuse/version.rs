use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    clock,
    record::{Envelope, RecordDraft},
    transaction::{ReceiptHalf, verify_quote},
};

use super::helpers::{build_and_sign_enquiry, build_and_sign_quote, sample_terms_input};
use crate::app::{
    AGREEMENTS, AgreementRow, ENQUIRY_HISTORY, LEDGER,
    agreement_ops::{self, AgreementAcceptInput},
    decline_ops, put_bytes, put_row,
    quote_ops::{self, QuoteSetInput, store_received_quote},
    tests::fixture,
};

#[tokio::test]
async fn accepting_a_superseded_version_is_refused() {
    let fix = fixture();
    let author = Identity::generate().expect("author identity");
    let counterparty_did = fix.master_did.clone();
    let now = clock::now_secs();

    // Author creates quote v1
    let (pay_q1, rec_q1, env_q1) =
        build_and_sign_quote(&author, &counterparty_did, 1, None, now + 3600, now);
    let v_q1 = verify_quote(&env_q1, now);
    assert!(v_q1.verified, "v_q1 must verify: {:?}", v_q1.reason);
    store_received_quote(&fix.host, &env_q1, &v_q1, &pay_q1.signed, now, &counterparty_did)
        .await
        .expect("store quote v1");
    crate::app::disclosures::store(
        &fix.host,
        &rec_q1,
        syneroym_roym_core::record::RECORD_QUOTE,
        &pay_q1.disclosures,
        now,
    )
    .await
    .unwrap();

    // Author creates quote v2 superseding v1 (same quote_id sequence 1)
    let (pay_q2, rec_q2, env_q2) = build_and_sign_quote(
        &author,
        &counterparty_did,
        1,
        Some(rec_q1.clone()),
        now + 3600,
        now + 10,
    );
    let v_q2 = verify_quote(&env_q2, now + 10);
    assert!(v_q2.verified, "v_q2 must verify: {:?}", v_q2.reason);
    store_received_quote(&fix.host, &env_q2, &v_q2, &pay_q2.signed, now + 10, &counterparty_did)
        .await
        .expect("store quote v2");
    crate::app::disclosures::store(
        &fix.host,
        &rec_q2,
        syneroym_roym_core::record::RECORD_QUOTE,
        &pay_q2.disclosures,
        now + 10,
    )
    .await
    .unwrap();

    // Counterparty tries to accept superseded v1
    let accept_v1_err = agreement_ops::accept_at(
        &fix.host,
        AgreementAcceptInput {
            quote_record_id: rec_q1,
            viewed_lang: None,
            viewed_via_mt: Some(false),
        },
        now + 20,
    )
    .await
    .unwrap_err();

    let err_str = serde_json::to_string(&accept_v1_err).expect("err json");
    assert!(err_str.contains("quote-superseded"));

    // Counterparty accepts latest v2
    let accept_v2 = agreement_ops::accept_at(
        &fix.host,
        AgreementAcceptInput {
            quote_record_id: rec_q2.clone(),
            viewed_lang: None,
            viewed_via_mt: Some(false),
        },
        now + 20,
    )
    .await
    .expect("accept v2 succeeds");

    assert_eq!(accept_v2.quote_record_id, rec_q2);
}

#[tokio::test]
async fn revision_after_acceptance_is_refused() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("counterparty identity");
    let counterparty_did = substrate::derive_did_key(&counterparty.public_key());
    let now = clock::now_secs();

    // Store enquiry from counterparty in ENQUIRY_HISTORY
    let (enq_rec, enq_env) = build_and_sign_enquiry(&counterparty, now);
    put_bytes(&fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes())
        .await
        .expect("store enquiry");

    // Author (this node) sets quote v1
    let quote_res = quote_ops::set_at(
        &fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.clone(),
            quote_id: None,
            expires_in_secs: 3600,
            terms: sample_terms_input("Fixing sink"),
        },
        now,
    )
    .await
    .expect("set quote v1");

    let quote_id = quote_res.quote_id.clone();
    let q_rec_id = quote_res.record_id.clone();

    // Counterparty accepts quote v1 (simulate receiving acceptance half)
    let (mut half_env, half_bytes) = Envelope::unsigned(
        RecordDraft {
            version: syneroym_roym_core::transaction::AGREEMENT_RECEIPT_VERSION,
            record_type: syneroym_roym_core::record::RECORD_AGREEMENT_RECEIPT.to_string(),
            subject: q_rec_id.clone(),
            payload: json!({
                "quote_record_id": q_rec_id,
                "role": "counterparty",
                "viewed_lang": "en",
                "viewed_via_mt": false,
            }),
            expires_at_secs: None,
            supersedes: None,
        },
        counterparty_did.clone(),
        None,
        now + 10,
    )
    .expect("draft half");
    let sig = counterparty.sign(&half_bytes).to_bytes();
    half_env.attach_signature(z32::encode(&sig)).expect("attach sig");
    let half_rec = half_env.record_id().expect("rec id");
    let half_json = half_env.to_json().expect("json");

    let agr_row = AgreementRow {
        quote_record_id: q_rec_id.clone(),
        quote_id: quote_id.clone(),
        conversation: "conv_1".to_string(),
        counterparty_did: counterparty_did.clone(),
        quote_author_did: fix.master_did.clone(),
        terms: sample_terms_input("Fixing sink").into_deal_terms(now + 3600),
        agreed_at_secs: None,
        counterparty: Some(ReceiptHalf {
            record_id: half_rec,
            envelope: half_json,
            issuer: counterparty_did,
            issued_at_secs: now + 10,
        }),
        quote_author: None,
        state: None,
        state_reason: None,
        cancel_reason: None,
        seq: None,
        commitments: vec![],
        updated_at_secs: now + 10,
    };
    put_row(&fix.host, AGREEMENTS, &q_rec_id, &agr_row).await.expect("put agr");

    // Author tries to revise the quote
    let revise_err = quote_ops::set_at(
        &fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec,
            quote_id: Some(quote_id),
            expires_in_secs: 3600,
            terms: sample_terms_input("Fixing sink and pipe"),
        },
        now + 20,
    )
    .await
    .unwrap_err();

    let err_str = serde_json::to_string(&revise_err).expect("err json");
    assert!(err_str.contains("quote-already-accepted"));
}

#[tokio::test]
async fn revision_after_an_end_is_refused() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("counterparty identity");
    let now = clock::now_secs();

    // Store enquiry from counterparty in ENQUIRY_HISTORY
    let (enq_rec, enq_env) = build_and_sign_enquiry(&counterparty, now);
    put_bytes(&fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes())
        .await
        .expect("store enquiry");

    // Author sets quote v1
    let quote_res = quote_ops::set_at(
        &fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.clone(),
            quote_id: None,
            expires_in_secs: 3600,
            terms: sample_terms_input("Fixing sink"),
        },
        now,
    )
    .await
    .expect("set quote v1");

    let quote_id = quote_res.quote_id.clone();

    // Test case 1: Deal expired by clock
    let revise_expired_err = quote_ops::set_at(
        &fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.clone(),
            quote_id: Some(quote_id.clone()),
            expires_in_secs: 3600,
            terms: sample_terms_input("Fixing sink revision"),
        },
        now + 4000, // past expiry (3600s)
    )
    .await
    .unwrap_err();

    let err_exp_str = serde_json::to_string(&revise_expired_err).expect("err json");
    assert!(err_exp_str.contains("quote-ended"));

    // Test case 2: Deal ended by decline
    let (enq_rec2, enq_env2) = build_and_sign_enquiry(&counterparty, now);
    put_bytes(&fix.host, ENQUIRY_HISTORY, &enq_rec2, enq_env2.as_bytes())
        .await
        .expect("store enquiry 2");

    let quote_res2 = quote_ops::set_at(
        &fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec2.clone(),
            quote_id: None,
            expires_in_secs: 3600,
            terms: sample_terms_input("Job 2"),
        },
        now,
    )
    .await
    .expect("set quote v2");

    let quote_id2 = quote_res2.quote_id.clone();
    let q2_rec_id = quote_res2.record_id.clone();

    // Mark declined
    let dec_key = decline_ops::decline_id(&quote_id2);
    put_row(
        &fix.host,
        LEDGER,
        &dec_key,
        &decline_ops::DeclineRow {
            kind: "decline".to_string(),
            deal: quote_id2.clone(),
            declines: q2_rec_id,
            record_id: "rec_dec".to_string(),
            envelope: "{}".to_string(),
            created_at_secs: now,
            message_id: None,
        },
    )
    .await
    .expect("put decline");

    let revise_declined_err = quote_ops::set_at(
        &fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec2,
            quote_id: Some(quote_id2),
            expires_in_secs: 3600,
            terms: sample_terms_input("Job 2 revised"),
        },
        now + 10,
    )
    .await
    .unwrap_err();

    let err_dec_str = serde_json::to_string(&revise_declined_err).expect("err json");
    assert!(err_dec_str.contains("quote-ended"));
}
