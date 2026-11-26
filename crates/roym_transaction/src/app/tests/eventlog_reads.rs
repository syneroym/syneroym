use syneroym_roym_core::{
    clock,
    deal::{CommitmentKind, LocalText, ResourceKind},
    lang::LangTag,
    transaction::DealRole,
};

use super::{TestFixture, fixture, refuse::helpers::build_and_sign_enquiry, settle::sample_note};
use crate::app::{
    ENQUIRY_HISTORY,
    agreement_ops::{self, AgreementAcceptInput},
    close_ops::{AgreementCloseInput, CloseAs, close_at},
    eventlog_ops::{self, EventLogListInput},
    fulfilment_ops::{self, FulfilmentSignInput},
    payment_ops::{self, PaymentAckInput},
    put_bytes,
    quote_ops::{self, CommitmentInput, DealTermsInput, QuoteSetInput},
    sync,
};

fn lesson_terms(payee: &str) -> DealTermsInput {
    let en = || LangTag::parse_canonical("en").expect("parse lang");
    DealTermsInput {
        currency: "EUR".to_string(),
        home_exchange: None,
        listing_id: None,
        scope: LocalText::plain(en(), "Lesson deal"),
        cancellation_terms: LocalText::plain(en(), "none"),
        refund_terms: LocalText::plain(en(), "none"),
        dispute_path: LocalText::plain(en(), "none"),
        commitments: vec![
            CommitmentInput {
                giver: DealRole::QuoteAuthor,
                resource_kind: ResourceKind::GoodsOrService,
                kind: CommitmentKind::Fulfilment,
                extension: None,
                after: vec![],
                description: LocalText::plain(en(), "Lesson"),
                declared_value_minor: 5000,
                details: None,
                window: None,
                location: None,
                payee: None,
                payment_methods: None,
                tax_minor: None,
                fees_minor: None,
                eep: None,
            },
            CommitmentInput {
                giver: DealRole::Counterparty,
                resource_kind: ResourceKind::Currency,
                kind: CommitmentKind::Currency,
                extension: None,
                after: vec![],
                description: LocalText::plain(en(), "Payment"),
                declared_value_minor: 5000,
                details: None,
                window: None,
                location: None,
                payee: Some(payee.to_string()),
                payment_methods: None,
                tax_minor: None,
                fees_minor: None,
                eep: None,
            },
        ],
    }
}

async fn relay_last_card(from: &TestFixture, to: &TestFixture, id: &str, at: u64) {
    let body = from.host.sent_cards().last().expect("a card to relay").clone();
    let msg = serde_json::json!({
        "id": id,
        "body": body,
        "sender_timestamp_ms": (at * 1000) as i64,
    });
    sync::file_incoming_card(&to.host, &msg, "conv_1", at, &to.master_did).await.unwrap();
}

async fn quote_and_countersign(
    author_fix: &TestFixture,
    cp_fix: &TestFixture,
    enq_rec: &str,
    now: u64,
) -> String {
    let quote_out = quote_ops::set_at(
        &author_fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.to_string(),
            quote_id: None,
            expires_in_secs: 3600,
            terms: lesson_terms(&author_fix.master_did),
        },
        now,
    )
    .await
    .expect("quote set");
    relay_last_card(author_fix, cp_fix, "msg_quote", now).await;

    agreement_ops::accept_at(
        &cp_fix.host,
        AgreementAcceptInput {
            quote_record_id: quote_out.record_id.clone(),
            viewed_lang: None,
            viewed_via_mt: None,
        },
        now + 5,
    )
    .await
    .expect("accept");
    relay_last_card(cp_fix, author_fix, "msg_acc", now + 5).await;
    sync::sync_direct(&author_fix.host, "conv_1", false, now + 10).await.unwrap();

    for (i, body) in author_fix.host.sent_cards().iter().skip(1).enumerate() {
        let msg = serde_json::json!({
            "id": format!("msg_author_card_{i}"),
            "body": body,
            "sender_timestamp_ms": ((now + 10) * 1000) as i64,
        });
        sync::file_incoming_card(&cp_fix.host, &msg, "conv_1", now + 10, &cp_fix.master_did)
            .await
            .unwrap();
    }
    quote_out.record_id
}

async fn settle_and_close(
    author_fix: &TestFixture,
    cp_fix: &TestFixture,
    agreement: &str,
    now: u64,
) {
    cp_fix.host.set_now(now + 15);
    fulfilment_ops::sign_at(
        &cp_fix.host,
        FulfilmentSignInput {
            agreement: agreement.to_string(),
            commitment_id: "c1".to_string(),
            settlement_note: Some(sample_note("Lesson done")),
        },
        now + 15,
    )
    .await
    .expect("sign fulfilment");
    relay_last_card(cp_fix, author_fix, "msg_f", now + 15).await;

    payment_ops::acknowledge_at(
        &author_fix.host,
        PaymentAckInput {
            agreement: agreement.to_string(),
            commitment_id: "c2".to_string(),
            observed_at_secs: None,
            method: None,
            reference: None,
            settlement_note: Some(sample_note("Payment acked")),
            supersedes: None,
        },
        now + 20,
    )
    .await
    .expect("ack payment");

    close_at(
        &author_fix.host,
        AgreementCloseInput {
            agreement: agreement.to_string(),
            close_as: CloseAs::Receiver,
            commitments: vec!["c2".to_string()],
        },
        now + 25,
    )
    .await
    .expect("close at");

    sync::sync_direct(&author_fix.host, "conv_1", false, now + 30).await.unwrap();
    sync::sync_direct(&cp_fix.host, "conv_1", false, now + 30).await.unwrap();
}

#[tokio::test]
async fn full_deal_path_never_reads_event_log_until_list_called() {
    let author_fix = fixture();
    let cp_fix = fixture();
    let now = clock::now_secs();

    let (enq_rec, enq_env) = build_and_sign_enquiry(&cp_fix.master, now);
    put_bytes(&author_fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();
    put_bytes(&cp_fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();

    author_fix.host.clear_reads();
    cp_fix.host.clear_reads();

    let agreement = quote_and_countersign(&author_fix, &cp_fix, &enq_rec, now).await;
    settle_and_close(&author_fix, &cp_fix, &agreement, now).await;

    let reads_author = author_fix.host.reads();
    let reads_cp = cp_fix.host.reads();
    assert!(
        reads_author.iter().all(|(coll, _)| coll != "event_log"),
        "found read of event_log during deal flow on author: {reads_author:?}"
    );
    assert!(
        reads_cp.iter().all(|(coll, _)| coll != "event_log"),
        "found read of event_log during deal flow on cp: {reads_cp:?}"
    );

    let list_out = eventlog_ops::list(&author_fix.host, EventLogListInput::default())
        .await
        .expect("list eventlog");
    assert!(!list_out.entries.is_empty());

    let reads_after_list = author_fix.host.reads();
    assert!(
        reads_after_list.iter().any(|(coll, _)| coll == "event_log"),
        "expected read of event_log when listing, but none found"
    );
}
