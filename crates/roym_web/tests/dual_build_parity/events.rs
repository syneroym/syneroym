use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;
use syneroym_core::event_bus::{Cursor, Event, EventBus, ReadOutcome};
use syneroym_roym_core::transaction::{
    AGREEMENT_RECEIPT_VERSION, DEFAULT_DATA_USE_NOTICE, RECORD_AGREEMENT_RECEIPT, RECORD_REQUEST,
    REQUEST_VERSION,
};
use syneroym_signed_record::Envelope;

use super::{deal_fixtures, fixtures::*, helpers::*};

fn current_cursor(bus: &EventBus) -> Cursor {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    match bus.read("roym", None, 100, now) {
        ReadOutcome::Events { last, .. } => last,
        ReadOutcome::Reload { now: head } => head,
    }
}

fn read_events_after(bus: &EventBus, cursor: &Cursor) -> (Vec<Event>, Cursor) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    match bus.read("roym", Some(cursor), 100, now) {
        ReadOutcome::Events { events, last } => (events, last),
        ReadOutcome::Reload { now: head } => {
            panic!("unexpected reload at cursor {head:?}")
        }
    }
}

#[tokio::test]
async fn sending_a_message_records_the_same_event_in_both_builds() {
    let h = harness().await;
    let conv_id = open_conv(&h, "did:key:zPeerEvents1").await;
    assert!(!conv_id.is_empty());

    let (sw, sn) = both_rpc(
        &h,
        "conversation.send",
        json!({ "conversation": conv_id, "body": "hello parity events" }),
    )
    .await;
    let wasm_msg_id = sw["result"]["message_id"].as_str().unwrap();
    let native_msg_id = sn["result"]["message_id"].as_str().unwrap();

    let wasm_start = Cursor { run_id: h.wasm_event_bus.run_id().to_string(), seq: 0 };
    let native_start = Cursor { run_id: h.native_event_bus.run_id().to_string(), seq: 0 };

    let (wasm_events, _) = read_events_after(&h.wasm_event_bus, &wasm_start);
    let (native_events, _) = read_events_after(&h.native_event_bus, &native_start);

    assert_eq!(wasm_events.len(), 1);
    assert_eq!(native_events.len(), 1);

    assert_eq!(wasm_events[0].kind, "conversation.message");
    assert_eq!(native_events[0].kind, "conversation.message");

    assert_eq!(wasm_events[0].service, "conversation");
    assert_eq!(native_events[0].service, "conversation");

    assert_eq!(wasm_events[0].subject, conv_id);
    assert_eq!(native_events[0].subject, conv_id);

    assert_eq!(wasm_events[0].item.as_deref(), Some(wasm_msg_id));
    assert_eq!(native_events[0].item.as_deref(), Some(native_msg_id));
}

#[tokio::test]
async fn a_request_card_records_a_card_event_in_both_builds() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let c_w = current_cursor(&h.wasm_event_bus);
    let c_n = current_cursor(&h.native_event_bus);

    let params = json!({
        "conversation": conv,
        "description": "Paint fence",
        "lang": "en",
        "data_use_notice": DEFAULT_DATA_USE_NOTICE,
    });
    let (rw, rn) = both_rpc(&h, "request.set", params).await;
    assert!(rw["result"]["request_id"].is_string());
    assert!(rn["result"]["request_id"].is_string());
    let wasm_msg_id = rw["result"]["message_id"].as_str().unwrap();
    let native_msg_id = rn["result"]["message_id"].as_str().unwrap();

    let (events_w, _) = read_events_after(&h.wasm_event_bus, &c_w);
    let (events_n, _) = read_events_after(&h.native_event_bus, &c_n);

    let kinds_w: Vec<&str> = events_w.iter().map(|e| e.kind.as_str()).collect();
    let kinds_n: Vec<&str> = events_n.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds_w, vec!["conversation.message", "transaction.card"]);
    assert_eq!(kinds_n, vec!["conversation.message", "transaction.card"]);

    assert_eq!(events_w[1].service, "transaction");
    assert_eq!(events_n[1].service, "transaction");
    assert_eq!(events_w[1].subject, conv);
    assert_eq!(events_n[1].subject, conv);
    assert_eq!(events_w[1].item.as_deref(), Some(wasm_msg_id));
    assert_eq!(events_n[1].item.as_deref(), Some(native_msg_id));
}

#[tokio::test]
async fn accepting_an_agreement_records_card_and_booking_events_in_both_builds() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (req_rec_id, _) = deal_fixtures::setup_peer_request(&h, &conv).await;
    let mut q_params = valid_quote_params(&conv, &req_rec_id);
    q_params["terms"]["payment_timing"] = json!("after-work");
    let (qw, _) = both_rpc(&h, "quote.set", q_params).await;
    let quote_rec_id = qw["result"]["record_id"].as_str().unwrap().to_string();
    let quote_id = qw["result"]["quote_id"].as_str().unwrap().to_string();

    let (get_q, _) = both_rpc(&h, "quote.get", json!({ "quote_id": quote_id })).await;
    let quote_env = Envelope::from_json(get_q["result"]["envelope"].as_str().unwrap()).unwrap();
    let terms = quote_env.payload["terms"].clone();

    let (agr_id, agr_env) = peer_counterparty_receipt(
        &conv,
        &quote_rec_id,
        &peer_did(),
        &owner_did(),
        terms.clone(),
        1_001_000,
    );
    deliver_peer_card(
        &h,
        &format!("m-agr-{agr_id}"),
        &conv,
        &peer_did(),
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &agr_env,
    )
    .await;

    let c_w = current_cursor(&h.wasm_event_bus);
    let c_n = current_cursor(&h.native_event_bus);

    both_rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    let (events_w, _) = read_events_after(&h.wasm_event_bus, &c_w);
    let (events_n, _) = read_events_after(&h.native_event_bus, &c_n);

    let normalize_items = |events: &[Event]| -> Vec<(String, String, String, Option<String>)> {
        let mut msg_map: HashMap<String, String> = HashMap::new();
        let mut next_id = 0;
        events
            .iter()
            .map(|e| {
                let item_norm = e.item.as_ref().map(|item| {
                    if item.starts_with("msg:") {
                        msg_map
                            .entry(item.clone())
                            .or_insert_with(|| {
                                let s = format!("<msg:{next_id}>");
                                next_id += 1;
                                s
                            })
                            .clone()
                    } else {
                        item.clone()
                    }
                });
                (e.kind.clone(), e.service.clone(), e.subject.clone(), item_norm)
            })
            .collect()
    };

    let tuples_w = normalize_items(&events_w);
    let tuples_n = normalize_items(&events_n);

    assert_eq!(tuples_w, tuples_n);
    let kinds: Vec<&str> = tuples_w.iter().map(|(k, ..)| k.as_str()).collect();
    assert!(kinds.contains(&"transaction.card"));
    assert!(kinds.contains(&"transaction.booking"));
}

#[tokio::test]
async fn syncing_an_inbound_card_records_one_card_event_and_a_second_sync_none() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (req_id, req_env) = peer_signed_request(&conv, 1, 1_000_000);
    deliver_peer_card(
        &h,
        &format!("m-req-{req_id}"),
        &conv,
        &peer_did(),
        RECORD_REQUEST,
        REQUEST_VERSION,
        &req_env,
    )
    .await;

    let c_w1 = current_cursor(&h.wasm_event_bus);
    let c_n1 = current_cursor(&h.native_event_bus);

    let (sw1, sn1) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw1["result"]["filed"], 1);
    assert_eq!(sn1["result"]["filed"], 1);

    let (events_w1, c_w2) = read_events_after(&h.wasm_event_bus, &c_w1);
    let (events_n1, c_n2) = read_events_after(&h.native_event_bus, &c_n1);

    let card_events_w1: Vec<_> =
        events_w1.iter().filter(|e| e.kind == "transaction.card").collect();
    let card_events_n1: Vec<_> =
        events_n1.iter().filter(|e| e.kind == "transaction.card").collect();
    assert_eq!(card_events_w1.len(), 1);
    assert_eq!(card_events_n1.len(), 1);

    let (sw2, sn2) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw2["result"]["filed"], 0);
    assert_eq!(sn2["result"]["filed"], 0);

    let (events_w2, _) = read_events_after(&h.wasm_event_bus, &c_w2);
    let (events_n2, _) = read_events_after(&h.native_event_bus, &c_n2);
    assert!(events_w2.is_empty());
    assert!(events_n2.is_empty());
}

#[tokio::test]
async fn a_transaction_import_records_no_event() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let params = json!({
        "conversation": conv,
        "description": "Request for import test",
        "lang": "en",
        "data_use_notice": DEFAULT_DATA_USE_NOTICE,
    });
    both_rpc(&h, "request.set", params).await;

    let (exp_w, exp_n) = both_rpc(&h, "transaction.export", json!({})).await;

    let c_w = current_cursor(&h.wasm_event_bus);
    let c_n = current_cursor(&h.native_event_bus);

    let imp_w = one_rpc(&h, true, "transaction.import", json!({ "bundle": exp_w["result"] })).await;
    let imp_n =
        one_rpc(&h, false, "transaction.import", json!({ "bundle": exp_n["result"] })).await;
    assert!(imp_w["result"].is_object(), "WASM import failed: {imp_w:?}");
    assert!(imp_n["result"].is_object(), "native import failed: {imp_n:?}");

    let (events_w, _) = read_events_after(&h.wasm_event_bus, &c_w);
    let (events_n, _) = read_events_after(&h.native_event_bus, &c_n);
    assert!(events_w.is_empty());
    assert!(events_n.is_empty());
}
