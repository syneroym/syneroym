//! Who may reach the inbox: the app's answer to "may this message be shown?",
//! seen through history and the host's stored rows on both builds.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::json;
use syneroym_app_orchestration::{
    AppInstanceId, AppRegistry, LogicalServiceName, ServiceId, TopologyEntry, TopologyEpoch,
    TopologyKey, TopologyMode,
};
use syneroym_conversation::store::ConversationStore;
use syneroym_data_db::host_store::RecordWriteValue;
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

#[tokio::test]
async fn scenario_224_a_blocked_stranger_draws_the_same_reports_as_any_other() {
    let h = harness().await;
    both_rpc(&h, "contacts.set-limits", json!({ "window_secs": 3600, "max_per_window": 100 }))
        .await;
    both_rpc(&h, "block.add", json!({ "person_did": STRANGER_B, "address": STRANGER_B })).await;

    // Under the limit, each stranger sends one message and then many more.
    for (conv, author) in [("conv-224-a", STRANGER_A), ("conv-224-b", STRANGER_B)] {
        for i in 0..4 {
            deliver_both(&h, &format!("m-224-{conv}-{i}"), conv, author, 1_000 + i).await;
        }
    }

    // Neither is ever reported back: the first message made the chat
    // accepted for both, so nothing is charged and nothing is refused.
    for conv in ["conv-224-a", "conv-224-b"] {
        for i in 0..4 {
            let (_, reason, report) = stored_admission(&h, &format!("m-224-{conv}-{i}")).await;
            assert!(!report, "{conv} #{i}: nobody is told they were refused");
            assert_ne!(reason.as_deref(), Some("rate-limited"), "{conv} #{i}");
        }
    }
    assert_eq!(visible_count(&h, "conv-224-a").await, 4);
    assert_eq!(visible_count(&h, "conv-224-b").await, 0);
}

#[tokio::test]
async fn scenario_225_a_block_set_while_a_group_was_hidden_still_applies_on_unhide() {
    let h = harness().await;
    let (gw, gn) = both_rpc(&h, "group.create", json!({ "name": "Hidden 225" })).await;
    let group_w = gw["result"]["conversation_id"].as_str().unwrap().to_string();
    let group_n = gn["result"]["conversation_id"].as_str().unwrap().to_string();
    for (wasm, group) in [(true, &group_w), (false, &group_n)] {
        one_rpc(&h, wasm, "group.hide", json!({ "conversation": group })).await;
        h.deliver(wasm, inbound("m-225", group, STRANGER_B, 1_000, "while hidden")).await;
    }
    assert_eq!(stored_admission(&h, "m-225").await.0, "held");

    both_rpc(&h, "block.add", json!({ "person_did": STRANGER_B, "address": STRANGER_B })).await;
    one_rpc(&h, true, "group.unhide", json!({ "conversation": group_w })).await;
    one_rpc(&h, false, "group.unhide", json!({ "conversation": group_n })).await;
    h.ask_undecided_now().await;

    let (state, reason, report) = stored_admission(&h, "m-225").await;
    assert_eq!((state.as_str(), reason.as_deref(), report), ("dropped", Some("blocked"), false));
}

#[tokio::test]
async fn scenario_226_a_profile_outage_leaves_the_message_waiting() {
    let h = harness_with_unbound(Some("profile")).await;
    deliver_both(&h, "m-226", "conv-226", STRANGER_A, 1_000).await;

    // The inbox could not reach the block list, so it did not answer. The
    // message waits, unread, and is asked about again once the list is back.
    let (state, reason, report) = stored_admission(&h, "m-226").await;
    assert_eq!((state.as_str(), reason, report), ("undecided", None, false));
    assert_eq!(visible_count(&h, "conv-226").await, 0);

    bring_back(&h, "profile");
    let svc = did_for_service("conversation");
    for conv in [&h.wasm_conversation, &h.native_conversation] {
        let store = conv.store_for(&svc).await.unwrap();
        store.update_undecided_retry("m-226", 1, 0).unwrap();
    }
    h.ask_undecided_now().await;
    assert_eq!(stored_admission(&h, "m-226").await.0, "accepted");
    assert_eq!(visible_count(&h, "conv-226").await, 1);
}

/// Makes a service the harness left unreachable reachable again, on both
/// builds, as when it comes back after an outage.
fn bring_back(h: &Harness, name: &str) {
    for inventory in &h.inventories {
        inventory.register(
            TopologyKey::local(AppInstanceId::new("roym"), LogicalServiceName::new(name)),
            TopologyEntry {
                mode: TopologyMode::Singleton,
                members: vec![ServiceId::new(did_for_service(name))],
                sharding_strategy: None,
                epoch: TopologyEpoch(1),
                cache_ttl: Duration::from_secs(60),
                not_after: None,
            },
        );
    }
}

async fn seed_row(h: &Harness, wasm: bool, collection: &str, id: &str, row: serde_json::Value) {
    let (storage, ks) =
        if wasm { (&h.wasm_storage, &h.wasm_ks) } else { (&h.native_storage, &h.native_ks) };
    let db = storage.open_service_db(&did_for_service("conversation"), ks).await.unwrap();
    let write = RecordWriteValue { id: id.to_string(), payload: serde_json::to_vec(&row).unwrap() };
    db.put(collection, &write, "seed", None).await.unwrap();
}

async fn old_charge_rows(h: &Harness, wasm: bool) -> usize {
    let rows = h.conv_rows(wasm, "first_contact_charges").await;
    rows.iter().filter(|r| r["marker"] == "seeded-old").count()
}

#[tokio::test]
async fn scenario_227_old_first_contact_charges_are_pruned_at_most_once_an_hour() {
    let h = harness().await;
    // A first message creates the collections on both builds.
    deliver_both(&h, "m-227-first", "conv-227-a", STRANGER_A, 1_000).await;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let old = json!({ "marker": "seeded-old", "at_secs": 0, "admission": "allow" });

    for wasm in [true, false] {
        // The last prune was just now: an old row survives the next message,
        // even though each WASM call starts from a fresh component.
        seed_row(&h, wasm, "first_contact_charges", "old-227", old.clone()).await;
        seed_row(&h, wasm, "conversation_meta", "charge-prune", json!({ "at_secs": now })).await;
    }
    deliver_both(&h, "m-227-second", "conv-227-b", STRANGER_A, 2_000).await;
    for wasm in [true, false] {
        assert_eq!(old_charge_rows(&h, wasm).await, 1, "pruned too early (wasm: {wasm})");
    }

    // The last prune was long ago: the next message prunes the old row.
    for wasm in [true, false] {
        seed_row(&h, wasm, "conversation_meta", "charge-prune", json!({ "at_secs": 0 })).await;
    }
    deliver_both(&h, "m-227-third", "conv-227-c", STRANGER_A, 3_000).await;
    for wasm in [true, false] {
        assert_eq!(old_charge_rows(&h, wasm).await, 0, "not pruned (wasm: {wasm})");
    }
}

#[tokio::test]
async fn scenario_228_a_group_first_seen_from_a_blocked_owner_starts_hidden_and_can_be_shown() {
    let h = harness().await;
    let owner = STRANGER_B;
    both_rpc(&h, "block.add", json!({ "person_did": owner, "address": owner })).await;
    let svc = did_for_service("conversation");
    for conv in [&h.wasm_conversation, &h.native_conversation] {
        let store = conv.store_for(&svc).await.unwrap();
        let conn = store.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        ConversationStore::get_or_create_group_shell(&tx, "grp-228", owner, 1, 1_000).unwrap();
        tx.commit().unwrap();
    }

    // Another member writes in the group the blocked person owns.
    deliver_both(&h, "m-228", "grp-228", STRANGER_A, 2_000).await;

    let (state, reason, _) = stored_admission(&h, "m-228").await;
    assert_eq!((state.as_str(), reason.as_deref()), ("held", Some("group-hidden")));
    let (lw, ln) = both_rpc(&h, "conversation.list", json!({ "kind": "group" })).await;
    for list in [lw, ln] {
        assert!(list["result"]["conversations"].as_array().unwrap().is_empty());
    }

    // Unblocking the owner and showing the group brings the message in.
    both_rpc(&h, "block.remove", json!({ "person_did": owner, "address": owner })).await;
    one_rpc(&h, true, "group.unhide", json!({ "conversation": "grp-228" })).await;
    one_rpc(&h, false, "group.unhide", json!({ "conversation": "grp-228" })).await;
    h.ask_undecided_now().await;
    assert_eq!(stored_admission(&h, "m-228").await.0, "accepted");
}
