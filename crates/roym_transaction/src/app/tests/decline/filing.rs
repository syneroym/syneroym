//! Tests for filing incoming deal-decline cards via conversation sync.

use super::*;

async fn setup_quote_for_decline_filing(
    f: &TestFixture,
    cp: &Identity,
    now: u64,
) -> (String, QuotePayload) {
    let cp_did = substrate::derive_did_key(&cp.public_key());
    let (q_json, q_rec_id, quote) = make_quote_envelope(&f.master, &cp_did, now);
    put_bytes(&f.host, QUOTE_HISTORY, &q_rec_id, q_json.as_bytes()).await.unwrap();

    let ptr = RecordPointerRow {
        envelope: q_json,
        record_id: q_rec_id.clone(),
        id: quote.quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        issuer: f.master_did.clone(),
        mine: true,
        updated_at_secs: now,
        version_count: 1,
        issued_at_secs: now,
        request_record_id: None,
        counterparty_did: Some(cp_did),
        declined_at_secs: None,
        decline_note: None,
    };
    put_row(&f.host, QUOTES, &quote.quote_id, &ptr).await.unwrap();
    (q_rec_id, quote)
}

#[tokio::test]
async fn decline_filing_refuses_third_party_signer() {
    let f = fixture();
    let now = f.host.now();
    let third_party = Identity::generate().expect("third_party");
    let cp = Identity::generate().expect("cp");
    let (q_rec_id, quote) = setup_quote_for_decline_filing(&f, &cp, now).await;

    let (bad_dec_json, _) =
        make_decline_envelope(&third_party, &q_rec_id, &quote.quote_id, "conv_1", now);
    let bad_card_body =
        card::card_body(RECORD_DEAL_DECLINE, DEAL_DECLINE_VERSION, &bad_dec_json, &[])
            .expect("card body");

    f.host.on_call("conversation", "conversation.changes", move |_| {
        Ok(json!({
            "result": {
                "messages": [
                    {
                        "id": "msg_bad_dec",
                        "conversation": "conv_1",
                        "author": "did:key:third",
                        "direction": "incoming",
                        "sender_timestamp_ms": (now * 1000) as i64,
                        "content_type": CARD_CONTENT_TYPE,
                        "languages": ["en"],
                        "body_encoding": "utf8",
                        "restored": false,
                        "state": "delivered",
                        "stored_at_secs": now as i64,
                        "visible_seq": 1,
                        "verified": true,
                        "body": bad_card_body,
                    }
                ],
                "last_seq": 1,
                "kind": "direct"
            }
        })
        .to_string())
    });

    let sync_res =
        sync::sync(&f.host, SyncInput { conversation: "conv_1".to_string(), full: false })
            .await
            .unwrap();
    assert_eq!(sync_res.refused, 1);

    let card: CardRow = get_row(&f.host, CARDS, "msg_bad_dec").await.unwrap().unwrap();
    assert_eq!(card.reason.as_deref(), Some("only the counterparty can decline this quote"));
}

#[tokio::test]
async fn decline_filing_refuses_mismatched_conversation() {
    let f = fixture();
    let now = f.host.now();
    let cp = Identity::generate().expect("cp");
    let (q_rec_id, quote) = setup_quote_for_decline_filing(&f, &cp, now).await;

    let (bad_conv_json, _) =
        make_decline_envelope(&cp, &q_rec_id, &quote.quote_id, "other_conv", now);
    let bad_conv_body =
        card::card_body(RECORD_DEAL_DECLINE, DEAL_DECLINE_VERSION, &bad_conv_json, &[])
            .expect("card body");

    f.host.on_call("conversation", "conversation.changes", move |_| {
        Ok(json!({
            "result": {
                "messages": [
                    {
                        "id": "msg_bad_conv",
                        "conversation": "conv_1",
                        "author": "did:key:cp",
                        "direction": "incoming",
                        "sender_timestamp_ms": (now * 1000) as i64,
                        "content_type": CARD_CONTENT_TYPE,
                        "languages": ["en"],
                        "body_encoding": "utf8",
                        "restored": false,
                        "state": "delivered",
                        "stored_at_secs": now as i64,
                        "visible_seq": 1,
                        "verified": true,
                        "body": bad_conv_body,
                    }
                ],
                "last_seq": 1,
                "kind": "direct"
            }
        })
        .to_string())
    });

    let sync_res =
        sync::sync(&f.host, SyncInput { conversation: "conv_1".to_string(), full: false })
            .await
            .unwrap();
    assert_eq!(sync_res.refused, 1);
    let card: CardRow = get_row(&f.host, CARDS, "msg_bad_conv").await.unwrap().unwrap();
    assert_eq!(card.reason.as_deref(), Some("card names another conversation"));
}

#[tokio::test]
async fn decline_filing_refuses_mismatched_deal() {
    let f = fixture();
    let now = f.host.now();
    let cp = Identity::generate().expect("cp");
    let (q_rec_id, _quote) = setup_quote_for_decline_filing(&f, &cp, now).await;

    let (bad_deal_json, _) = make_decline_envelope(&cp, &q_rec_id, "wrong_quote_id", "conv_1", now);
    let bad_deal_body =
        card::card_body(RECORD_DEAL_DECLINE, DEAL_DECLINE_VERSION, &bad_deal_json, &[])
            .expect("card body");

    f.host.on_call("conversation", "conversation.changes", move |_| {
        Ok(json!({
            "result": {
                "messages": [
                    {
                        "id": "msg_bad_deal",
                        "conversation": "conv_1",
                        "author": "did:key:cp",
                        "direction": "incoming",
                        "sender_timestamp_ms": (now * 1000) as i64,
                        "content_type": CARD_CONTENT_TYPE,
                        "languages": ["en"],
                        "body_encoding": "utf8",
                        "restored": false,
                        "state": "delivered",
                        "stored_at_secs": now as i64,
                        "visible_seq": 1,
                        "verified": true,
                        "body": bad_deal_body,
                    }
                ],
                "last_seq": 1,
                "kind": "direct"
            }
        })
        .to_string())
    });

    let sync_res =
        sync::sync(&f.host, SyncInput { conversation: "conv_1".to_string(), full: false })
            .await
            .unwrap();
    assert_eq!(sync_res.refused, 1);
    let card: CardRow = get_row(&f.host, CARDS, "msg_bad_deal").await.unwrap().unwrap();
    assert_eq!(card.reason.as_deref(), Some("deal does not match quote_id"));
}

#[tokio::test]
async fn decline_filing_refuses_when_unrefused_half_exists() {
    let f = fixture();
    let now = f.host.now();
    let cp = Identity::generate().expect("cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());
    let (q_rec_id, quote) = setup_quote_for_decline_filing(&f, &cp, now).await;

    let agr_row = AgreementRow {
        quote_record_id: q_rec_id.clone(),
        quote_id: quote.quote_id.clone(),
        conversation: "conv_1".to_string(),
        counterparty_did: cp_did.clone(),
        quote_author_did: f.master_did.clone(),
        terms: quote.terms.clone(),
        agreed_at_secs: None,
        counterparty: Some(ReceiptHalf {
            record_id: "rec_cp_half".to_string(),
            envelope: "{}".to_string(),
            issuer: cp_did.clone(),
            issued_at_secs: now,
        }),
        quote_author: None,
        state: None,
        state_reason: None,
        cancel_reason: None,
        seq: None,
        commitments: vec![],
        updated_at_secs: now,
    };
    put_row(&f.host, AGREEMENTS, &q_rec_id, &agr_row).await.unwrap();

    let (unrefused_dec_json, _) =
        make_decline_envelope(&cp, &q_rec_id, &quote.quote_id, "conv_1", now);
    let unrefused_body =
        card::card_body(RECORD_DEAL_DECLINE, DEAL_DECLINE_VERSION, &unrefused_dec_json, &[])
            .expect("card body");

    f.host.on_call("conversation", "conversation.changes", move |_| {
        Ok(json!({
            "result": {
                "messages": [
                    {
                        "id": "msg_unrefused",
                        "conversation": "conv_1",
                        "author": "did:key:cp",
                        "direction": "incoming",
                        "sender_timestamp_ms": (now * 1000) as i64,
                        "content_type": CARD_CONTENT_TYPE,
                        "languages": ["en"],
                        "body_encoding": "utf8",
                        "restored": false,
                        "state": "delivered",
                        "stored_at_secs": now as i64,
                        "visible_seq": 1,
                        "verified": true,
                        "body": unrefused_body,
                    }
                ],
                "last_seq": 1,
                "kind": "direct"
            }
        })
        .to_string())
    });

    let sync_res =
        sync::sync(&f.host, SyncInput { conversation: "conv_1".to_string(), full: false })
            .await
            .unwrap();
    assert_eq!(sync_res.refused, 1);
    let card: CardRow = get_row(&f.host, CARDS, "msg_unrefused").await.unwrap().unwrap();
    assert_eq!(card.reason.as_deref(), Some("cannot decline a quote with an unrefused acceptance"));
}

#[tokio::test]
async fn decline_filing_retries_on_write_failure() {
    let f = fixture();
    let now = f.host.now();
    let cp = Identity::generate().expect("cp");
    let (q_rec_id, quote) = setup_quote_for_decline_filing(&f, &cp, now).await;

    let (dec_json, _) = make_decline_envelope(&cp, &q_rec_id, &quote.quote_id, "conv_1", now);
    let dec_body = card::card_body(RECORD_DEAL_DECLINE, DEAL_DECLINE_VERSION, &dec_json, &[])
        .expect("card body");

    f.host.on_call("conversation", "conversation.changes", move |_| {
        Ok(json!({
            "result": {
                "messages": [
                    {
                        "id": "msg_dec_retry",
                        "conversation": "conv_1",
                        "author": "did:key:cp",
                        "direction": "incoming",
                        "sender_timestamp_ms": (now * 1000) as i64,
                        "content_type": CARD_CONTENT_TYPE,
                        "languages": ["en"],
                        "body_encoding": "utf8",
                        "restored": false,
                        "state": "delivered",
                        "stored_at_secs": now as i64,
                        "visible_seq": 1,
                        "verified": true,
                        "body": dec_body,
                    }
                ],
                "last_seq": 1,
                "kind": "direct"
            }
        })
        .to_string())
    });

    // Injected storage failure on QUOTES write
    f.host.fail_write(QUOTES, &quote.quote_id);

    // First sync fails because of the write error
    let sync_err =
        sync::sync(&f.host, SyncInput { conversation: "conv_1".to_string(), full: false }).await;
    assert!(sync_err.is_err(), "sync must fail on storage error");
    assert!(f.host.was_failed_write_hit(), "failing write must have fired");

    // The card is NOT recorded as refused in CARDS
    let card: Option<CardRow> = get_row(&f.host, CARDS, "msg_dec_retry").await.unwrap();
    assert!(card.is_none(), "card must not be filed as refused on transient storage error");

    // Quote pointer is not declined yet
    let ptr_before: RecordPointerRow =
        get_row(&f.host, QUOTES, &quote.quote_id).await.unwrap().unwrap();
    assert_eq!(ptr_before.declined_at_secs, None);

    // Second sync without failing write succeeds and files the card as verified
    let sync_res =
        sync::sync(&f.host, SyncInput { conversation: "conv_1".to_string(), full: false })
            .await
            .expect("second sync must succeed");
    assert_eq!(sync_res.filed, 1);
    assert_eq!(sync_res.refused, 0);

    let card_after: CardRow = get_row(&f.host, CARDS, "msg_dec_retry").await.unwrap().unwrap();
    assert!(card_after.verified, "card must be filed as verified");
    assert_eq!(card_after.reason, None);

    let ptr_after: RecordPointerRow =
        get_row(&f.host, QUOTES, &quote.quote_id).await.unwrap().unwrap();
    assert_eq!(ptr_after.declined_at_secs, Some(now));
}
