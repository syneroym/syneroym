//! Who may reach the inbox: the app's answer to "may this message be shown?",
//! seen through history and the host's stored rows on both builds.

use serde_json::json;
use syneroym_rpc::{ConversationError, ConversationHost};

use super::{fixtures::*, helpers::*};

const STRANGER_A: &str = "did:key:zStrangerA216";
const STRANGER_B: &str = "did:key:zStrangerB216";

async fn deliver_both(h: &Harness, id: &str, conversation: &str, author: &str, ts: i64) {
    h.deliver(true, inbound(id, conversation, author, ts, "hello")).await;
    h.deliver(false, inbound(id, conversation, author, ts, "hello")).await;
}

async fn visible_count(h: &Harness, conversation: &str) -> usize {
    let (w, n) = both_rpc(h, "conversation.history", json!({ "conversation": conversation })).await;
    let (w, n) = (
        w["result"]["messages"].as_array().unwrap().len(),
        n["result"]["messages"].as_array().unwrap().len(),
    );
    assert_eq!(w, n, "both builds must show the same messages");
    w
}

async fn stored_admission(h: &Harness, id: &str) -> (String, Option<String>, bool) {
    let svc = did_for_service("conversation");
    let mut rows = Vec::new();
    for conv in [&h.wasm_conversation, &h.native_conversation] {
        let m = conv.store_for(&svc).await.unwrap().get_message(id).unwrap().unwrap();
        rows.push((m.admission, m.admission_reason, m.report_refusal));
    }
    assert_eq!(rows[0], rows[1], "both builds must store the same answer");
    rows.remove(0)
}

#[tokio::test]
async fn scenario_216_a_reply_in_an_opened_chat_is_not_a_first_contact() {
    let h = harness().await;
    both_rpc(&h, "contacts.set-limits", json!({ "window_secs": 3600, "max_per_window": 1 })).await;
    let conv = open_conv(&h, STRANGER_A).await;

    deliver_both(&h, "m-216-reply", &conv, STRANGER_A, 1_000).await;
    assert_eq!(visible_count(&h, &conv).await, 1);

    // The reply was not charged: the one first contact this person is
    // allowed in the window is still unused.
    deliver_both(&h, "m-216-new", "conv-216-b", STRANGER_A, 2_000).await;
    assert_eq!(visible_count(&h, "conv-216-b").await, 1);
}

#[tokio::test]
async fn scenario_217_block_and_unblock_change_what_reaches_the_inbox() {
    let h = harness().await;
    let params = json!({ "person_did": STRANGER_A, "address": STRANGER_A });
    both_rpc(&h, "block.add", params.clone()).await;

    deliver_both(&h, "m-217-blocked", "conv-217", STRANGER_A, 1_000).await;
    let (state, reason, report) = stored_admission(&h, "m-217-blocked").await;
    assert_eq!((state.as_str(), reason.as_deref(), report), ("dropped", Some("blocked"), false));

    both_rpc(&h, "block.remove", params).await;
    deliver_both(&h, "m-217-after", "conv-217", STRANGER_A, 2_000).await;
    assert_eq!(visible_count(&h, "conv-217").await, 1);
}

#[tokio::test]
async fn scenario_218_asking_again_about_one_message_charges_once() {
    let h = harness().await;
    both_rpc(&h, "contacts.set-limits", json!({ "window_secs": 3600, "max_per_window": 2 })).await;
    deliver_both(&h, "m-218", "conv-218-a", STRANGER_A, 1_000).await;

    // The app is asked about the same message a second time, as the
    // background pass does when an answer was lost.
    let svc = did_for_service("conversation");
    for conv in [&h.wasm_conversation, &h.native_conversation] {
        let store = conv.store_for(&svc).await.unwrap();
        let guard = store.conn().lock().unwrap();
        guard
            .execute(
                "UPDATE messages SET admission = 'undecided', visible_seq = 0, next_notify_at = 0 \
                 WHERE id = 'm-218'",
                [],
            )
            .unwrap();
    }
    h.ask_undecided_now().await;
    assert_eq!(stored_admission(&h, "m-218").await.0, "accepted");

    // One charge was spent, so a second first contact fits in the limit of two.
    deliver_both(&h, "m-218-b", "conv-218-b", STRANGER_A, 2_000).await;
    assert_eq!(visible_count(&h, "conv-218-b").await, 1);
}

#[tokio::test]
async fn scenario_219_only_a_rate_limit_is_reported_back_to_the_sender() {
    let h = harness().await;
    both_rpc(&h, "contacts.set-limits", json!({ "window_secs": 3600, "max_per_window": 1 })).await;
    both_rpc(&h, "block.add", json!({ "person_did": STRANGER_B, "address": STRANGER_B })).await;

    deliver_both(&h, "m-219-first", "conv-219-a", STRANGER_A, 1_000).await;
    deliver_both(&h, "m-219-limited", "conv-219-c", STRANGER_A, 2_000).await;
    deliver_both(&h, "m-219-blocked", "conv-219-b", STRANGER_B, 3_000).await;

    let (state, reason, report) = stored_admission(&h, "m-219-limited").await;
    assert_eq!(
        (state.as_str(), reason.as_deref(), report),
        ("dropped", Some("rate-limited"), true)
    );
    let (state, reason, report) = stored_admission(&h, "m-219-blocked").await;
    assert_eq!((state.as_str(), reason.as_deref(), report), ("dropped", Some("blocked"), false));

    // A blocked person uses up the limit at the same count as anyone else,
    // so the sender cannot tell from the reports that they are blocked.
    deliver_both(&h, "m-219-blocked-again", "conv-219-d", STRANGER_B, 4_000).await;
    let (state, reason, report) = stored_admission(&h, "m-219-blocked-again").await;
    assert_eq!(
        (state.as_str(), reason.as_deref(), report),
        ("dropped", Some("rate-limited"), true)
    );
}

#[tokio::test]
async fn scenario_220_a_message_held_for_a_hidden_group_is_not_readable() {
    let h = harness().await;
    let (gw, gn) = both_rpc(&h, "group.create", json!({ "name": "Hidden 220" })).await;
    let group_w = gw["result"]["conversation_id"].as_str().unwrap().to_string();
    let group_n = gn["result"]["conversation_id"].as_str().unwrap().to_string();
    one_rpc(&h, true, "group.hide", json!({ "conversation": group_w })).await;
    one_rpc(&h, false, "group.hide", json!({ "conversation": group_n })).await;
    h.deliver(true, inbound("m-220", &group_w, STRANGER_A, 1_000, "secret plan")).await;
    h.deliver(false, inbound("m-220", &group_n, STRANGER_A, 1_000, "secret plan")).await;
    assert_eq!(stored_admission(&h, "m-220").await.0, "held");

    let svc = did_for_service("conversation");
    for conv in [&h.wasm_conversation, &h.native_conversation] {
        let err = conv.get_message(&svc, "m-220").await.unwrap_err();
        assert_eq!(err, ConversationError::NotFound);
    }
    let (sw, sn) = both_rpc(&h, "conversation.search", json!({ "query": "secret plan" })).await;
    assert!(sw["result"]["matches"].as_array().unwrap().is_empty());
    assert!(sn["result"]["matches"].as_array().unwrap().is_empty());
}
