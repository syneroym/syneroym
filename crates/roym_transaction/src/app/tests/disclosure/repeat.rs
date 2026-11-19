use serde_json::json;
use syneroym_roym_core::card;

use super::helpers::{build_and_sign_enquiry, sample_terms_input};
use crate::app::{
    CARDS, CardRow, DISCLOSURES, ENQUIRY_HISTORY,
    disclosures::DisclosureRow,
    get_row, put_bytes,
    quote_ops::{self, QuoteSetInput},
    sync,
    tests::fixture,
};

#[tokio::test]
async fn a_repeat_quote_card_with_the_same_set_is_a_repeat() {
    let author_fix = fixture();
    let cp_fix = fixture();
    let now = author_fix.host.now();

    let (enq_rec, enq_env) = build_and_sign_enquiry(&cp_fix.master, now);
    put_bytes(&author_fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();
    put_bytes(&cp_fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();

    let _res = quote_ops::set_at(
        &author_fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.clone(),
            quote_id: None,
            expires_in_secs: 3600,
            terms: sample_terms_input("Service terms"),
        },
        now,
    )
    .await
    .unwrap();

    let body = author_fix.host.sent_cards().last().unwrap().clone();
    let msg1 = json!({
        "id": "msg_quote_1",
        "body": body,
        "sender_timestamp_ms": (now * 1000) as i64,
    });
    let res1 = sync::file_incoming_card(&cp_fix.host, &msg1, "conv_1", now, &cp_fix.master_did)
        .await
        .unwrap();
    assert!(res1.filed);
    let card1: CardRow = get_row(&cp_fix.host, CARDS, "msg_quote_1").await.unwrap().unwrap();
    assert!(!card1.repeat);
    assert!(card1.verified);

    let msg2 = json!({
        "id": "msg_quote_repeat",
        "body": body,
        "sender_timestamp_ms": ((now + 5) * 1000) as i64,
    });
    let res2 = sync::file_incoming_card(&cp_fix.host, &msg2, "conv_1", now + 5, &cp_fix.master_did)
        .await
        .unwrap();
    assert!(res2.filed);
    let card2: CardRow = get_row(&cp_fix.host, CARDS, "msg_quote_repeat").await.unwrap().unwrap();
    assert!(card2.repeat);
    assert!(card2.verified);

    let mut card_parsed = card::parse_card(&body).unwrap();
    card_parsed.disclosures.reverse();
    let reversed_body = card::card_body(
        &card_parsed.card_type,
        card_parsed.version,
        &card_parsed.envelope,
        &card_parsed.disclosures,
    )
    .unwrap();

    let msg3 = json!({
        "id": "msg_quote_repeat_rev",
        "body": reversed_body,
        "sender_timestamp_ms": ((now + 10) * 1000) as i64,
    });
    let res3 =
        sync::file_incoming_card(&cp_fix.host, &msg3, "conv_1", now + 10, &cp_fix.master_did)
            .await
            .unwrap();
    assert!(res3.filed);
    let card3: CardRow =
        get_row(&cp_fix.host, CARDS, "msg_quote_repeat_rev").await.unwrap().unwrap();
    assert!(card3.repeat);
    assert!(card3.verified);
}

#[tokio::test]
async fn a_repeat_quote_card_without_disclosures_uses_the_stored_set() {
    let author_fix = fixture();
    let cp_fix = fixture();
    let now = author_fix.host.now();

    let (enq_rec, enq_env) = build_and_sign_enquiry(&cp_fix.master, now);
    put_bytes(&author_fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();
    put_bytes(&cp_fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();

    let _res = quote_ops::set_at(
        &author_fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.clone(),
            quote_id: None,
            expires_in_secs: 3600,
            terms: sample_terms_input("Service terms"),
        },
        now,
    )
    .await
    .unwrap();

    let body = author_fix.host.sent_cards().last().unwrap().clone();
    let msg1 = json!({
        "id": "msg_quote_1",
        "body": body,
        "sender_timestamp_ms": (now * 1000) as i64,
    });
    sync::file_incoming_card(&cp_fix.host, &msg1, "conv_1", now, &cp_fix.master_did).await.unwrap();

    let card_parsed = card::parse_card(&body).unwrap();
    let empty_disc_body =
        card::card_body(&card_parsed.card_type, card_parsed.version, &card_parsed.envelope, &[])
            .unwrap();

    let msg2 = json!({
        "id": "msg_quote_empty_disc",
        "body": empty_disc_body,
        "sender_timestamp_ms": ((now + 5) * 1000) as i64,
    });
    let res2 = sync::file_incoming_card(&cp_fix.host, &msg2, "conv_1", now + 5, &cp_fix.master_did)
        .await
        .unwrap();
    assert!(res2.filed);
    let card2: CardRow =
        get_row(&cp_fix.host, CARDS, "msg_quote_empty_disc").await.unwrap().unwrap();
    assert!(card2.repeat);
    assert!(card2.verified);
    assert!(card2.data.is_some());
    let data = card2.data.unwrap();
    assert_eq!(data["terms"]["currency"], "EUR");
}

#[tokio::test]
async fn a_repeat_with_a_bad_set_is_refused_and_the_stored_set_stays() {
    let author_fix = fixture();
    let cp_fix = fixture();
    let now = author_fix.host.now();

    let (enq_rec, enq_env) = build_and_sign_enquiry(&cp_fix.master, now);
    put_bytes(&author_fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();
    put_bytes(&cp_fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();

    let set_out = quote_ops::set_at(
        &author_fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.clone(),
            quote_id: None,
            expires_in_secs: 3600,
            terms: sample_terms_input("Service terms"),
        },
        now,
    )
    .await
    .unwrap();

    let body = author_fix.host.sent_cards().last().unwrap().clone();
    let msg1 = json!({
        "id": "msg_quote_1",
        "body": body,
        "sender_timestamp_ms": (now * 1000) as i64,
    });
    sync::file_incoming_card(&cp_fix.host, &msg1, "conv_1", now, &cp_fix.master_did).await.unwrap();

    let stored_before: DisclosureRow =
        get_row(&cp_fix.host, DISCLOSURES, &set_out.record_id).await.unwrap().unwrap();

    let mut card_parsed = card::parse_card(&body).unwrap();
    card_parsed.disclosures.pop();
    let bad_body = card::card_body(
        &card_parsed.card_type,
        card_parsed.version,
        &card_parsed.envelope,
        &card_parsed.disclosures,
    )
    .unwrap();

    let msg2 = json!({
        "id": "msg_quote_bad_repeat",
        "body": bad_body,
        "sender_timestamp_ms": ((now + 5) * 1000) as i64,
    });
    let res2 = sync::file_incoming_card(&cp_fix.host, &msg2, "conv_1", now + 5, &cp_fix.master_did)
        .await
        .unwrap();
    assert!(res2.refused);

    let card2: CardRow =
        get_row(&cp_fix.host, CARDS, "msg_quote_bad_repeat").await.unwrap().unwrap();
    assert!(!card2.verified);

    let stored_after: DisclosureRow =
        get_row(&cp_fix.host, DISCLOSURES, &set_out.record_id).await.unwrap().unwrap();
    assert_eq!(stored_before.disclosures, stored_after.disclosures);
}
