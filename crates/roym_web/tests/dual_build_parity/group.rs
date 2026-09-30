use serde_json::{Value, json};
use syneroym_roym_core::{
    card::CARD_CONTENT_TYPE,
    conversation::{
        DELETION_REQUEST_CONTENT_TYPE, deletion_request_body,
        group::{
            CARDS_NOT_IN_GROUPS_MESSAGE, GROUP_PROFILE_CONTENT_TYPE, MEMBERSHIP_EVENT_CONTENT_TYPE,
            group_profile_body,
        },
    },
};
use syneroym_rpc::{ConversationDeliveryState, ConversationMessage};

use super::{fixtures::*, helpers::*};

fn inbound_custom(
    id: &str,
    conversation: &str,
    author: &str,
    ts: i64,
    content_type: &str,
    body: Vec<u8>,
) -> ConversationMessage {
    ConversationMessage {
        id: id.to_string(),
        conversation: conversation.to_string(),
        author: author.to_string(),
        sender_timestamp: ts,
        received_at: ts,
        content_type: content_type.to_string(),
        body,
        state: ConversationDeliveryState::Delivered,
        verified: true,
        last_error: None,
    }
}

#[tokio::test]
async fn scenario_201_group_create_with_name_parity() {
    let h = harness().await;
    let (gw, gn) = both_rpc(&h, "group.create", json!({ "name": "Garden Club" })).await;
    let group_w = gw["result"]["conversation_id"].as_str().unwrap().to_string();
    let group_n = gn["result"]["conversation_id"].as_str().unwrap().to_string();
    assert!(!group_w.is_empty());
    assert!(!group_n.is_empty());

    let lw = one_rpc(&h, true, "conversation.list", json!({ "kind": "group" })).await;
    let ln = one_rpc(&h, false, "conversation.list", json!({ "kind": "group" })).await;
    assert_eq!(lw["result"]["conversations"].as_array().unwrap().len(), 1);
    assert_eq!(ln["result"]["conversations"].as_array().unwrap().len(), 1);
    assert_eq!(lw["result"]["conversations"][0]["group"]["name"], "Garden Club");
    assert_eq!(ln["result"]["conversations"][0]["group"]["name"], "Garden Club");
    assert_eq!(lw["result"]["conversations"][0]["kind"], "group");
    assert_eq!(ln["result"]["conversations"][0]["kind"], "group");

    let iw = one_rpc(&h, true, "group.info", json!({ "conversation": group_w })).await;
    let in_ = one_rpc(&h, false, "group.info", json!({ "conversation": group_n })).await;
    assert_eq!(iw["result"]["name"], "Garden Club");
    assert_eq!(in_["result"]["name"], "Garden Club");
    assert_eq!(iw["result"]["is_owner"], true);
    assert_eq!(in_["result"]["is_owner"], true);
    assert_eq!(iw["result"]["is_member"], true);
    assert_eq!(in_["result"]["is_member"], true);
    assert_eq!(iw["result"]["epoch"], 1);
    assert_eq!(in_["result"]["epoch"], 1);
    assert_eq!(iw["result"]["key_epoch"], 1);
    assert_eq!(in_["result"]["key_epoch"], 1);
    assert!(iw["result"]["notices"]["owner_can_read"].is_string());
    assert!(iw["result"]["notices"]["key_trust"].is_string());
    assert!(iw["result"]["notices"]["delivery"].is_string());
    assert!(iw["result"]["notices"]["join_boundary"].is_string());
    assert!(iw["result"]["notices"]["removed"].is_null());
    assert!(iw["result"]["notices"]["restored"].is_null());

    let hw = one_rpc(&h, true, "conversation.history", json!({ "conversation": group_w })).await;
    let hn = one_rpc(&h, false, "conversation.history", json!({ "conversation": group_n })).await;
    let msgs_w = hw["result"]["messages"].as_array().unwrap();
    let msgs_n = hn["result"]["messages"].as_array().unwrap();
    assert_eq!(msgs_w.len(), 1);
    assert_eq!(msgs_n.len(), 1);
    assert_eq!(msgs_w[0]["content_type"], MEMBERSHIP_EVENT_CONTENT_TYPE);
    assert_eq!(msgs_n[0]["content_type"], MEMBERSHIP_EVENT_CONTENT_TYPE);
    let body_w: Value = serde_json::from_str(msgs_w[0]["body"].as_str().unwrap()).unwrap();
    assert_eq!(body_w["action"], "add");
    assert_eq!(body_w["epoch"], 1);
}

#[tokio::test]
async fn scenario_202_group_rename_validation_parity() {
    let h = harness().await;
    let (gw, gn) = both_rpc(&h, "group.create", json!({ "name": "Initial" })).await;
    let group_w = gw["result"]["conversation_id"].as_str().unwrap().to_string();
    let group_n = gn["result"]["conversation_id"].as_str().unwrap().to_string();

    let cases = [
        ("", "a group name must not be empty"),
        ("   ", "a group name must not be empty"),
        (&"a".repeat(81), "a group name must be at most 80 characters"),
        ("bad\x07name", "a group name must not contain control characters"),
    ];

    for (name, expected_err) in cases {
        let rw =
            one_rpc(&h, true, "group.rename", json!({ "conversation": group_w, "name": name }))
                .await;
        let rn =
            one_rpc(&h, false, "group.rename", json!({ "conversation": group_n, "name": name }))
                .await;
        assert_eq!(rw["error"]["message"].as_str().unwrap(), expected_err);
        assert_eq!(rn["error"]["message"].as_str().unwrap(), expected_err);
    }

    let valid_80 = "a".repeat(80);
    let rw =
        one_rpc(&h, true, "group.rename", json!({ "conversation": group_w, "name": valid_80 }))
            .await;
    let rn =
        one_rpc(&h, false, "group.rename", json!({ "conversation": group_n, "name": valid_80 }))
            .await;
    assert_eq!(rw["result"]["name"], valid_80);
    assert_eq!(rn["result"]["name"], valid_80);
}

#[tokio::test]
async fn scenario_203_send_into_group_with_no_other_member_fails_parity() {
    let h = harness().await;
    let (gw, gn) = both_rpc(&h, "group.create", json!({ "name": "Alone" })).await;
    let group_w = gw["result"]["conversation_id"].as_str().unwrap().to_string();
    let group_n = gn["result"]["conversation_id"].as_str().unwrap().to_string();

    let sw =
        one_rpc(&h, true, "conversation.send", json!({ "conversation": group_w, "body": "hello" }))
            .await;
    let sn = one_rpc(
        &h,
        false,
        "conversation.send",
        json!({ "conversation": group_n, "body": "hello" }),
    )
    .await;
    assert!(sw["error"]["message"].as_str().unwrap().contains("nowhere to deliver"), "{sw}");
    assert!(sn["error"]["message"].as_str().unwrap().contains("nowhere to deliver"), "{sn}");

    let hw = one_rpc(&h, true, "conversation.history", json!({ "conversation": group_w })).await;
    let hn = one_rpc(&h, false, "conversation.history", json!({ "conversation": group_n })).await;
    assert_eq!(hw["result"]["messages"].as_array().unwrap().len(), 1);
    assert_eq!(hn["result"]["messages"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn scenario_204_two_inbound_messages_update_count_and_activity_parity() {
    let h = harness().await;
    let (gw, gn) = both_rpc(&h, "group.create", json!({ "name": "Active Group" })).await;
    let group_w = gw["result"]["conversation_id"].as_str().unwrap().to_string();
    let group_n = gn["result"]["conversation_id"].as_str().unwrap().to_string();
    let hw0 = one_rpc(&h, true, "conversation.history", json!({ "conversation": group_w })).await;
    let genesis_ts = hw0["result"]["messages"][0]["sender_timestamp_ms"].as_i64().unwrap();
    let ts1 = genesis_ts + 1_000;
    let ts2 = genesis_ts + 2_000;

    h.deliver(true, inbound("m-204a", &group_w, "did:key:zPeer204", ts1, "msg 1")).await;
    h.deliver(false, inbound("m-204a", &group_n, "did:key:zPeer204", ts1, "msg 1")).await;
    h.deliver(true, inbound("m-204b", &group_w, "did:key:zPeer204", ts2, "msg 2")).await;
    h.deliver(false, inbound("m-204b", &group_n, "did:key:zPeer204", ts2, "msg 2")).await;

    let lw = one_rpc(&h, true, "conversation.list", json!({ "kind": "group" })).await;
    let ln = one_rpc(&h, false, "conversation.list", json!({ "kind": "group" })).await;
    assert_eq!(lw["result"]["conversations"][0]["message_count"], 2);
    assert_eq!(ln["result"]["conversations"][0]["message_count"], 2);
    assert_eq!(lw["result"]["conversations"][0]["last_activity_ms"], ts2);
    assert_eq!(ln["result"]["conversations"][0]["last_activity_ms"], ts2);

    let hw = one_rpc(&h, true, "conversation.history", json!({ "conversation": group_w })).await;
    let hn = one_rpc(&h, false, "conversation.history", json!({ "conversation": group_n })).await;
    let msgs_w = hw["result"]["messages"].as_array().unwrap();
    let msgs_n = hn["result"]["messages"].as_array().unwrap();
    assert_eq!(msgs_w.len(), 3);
    assert_eq!(msgs_n.len(), 3);
    assert_eq!(msgs_w[1]["id"], "m-204a");
    assert_eq!(msgs_w[2]["id"], "m-204b");
    assert_eq!(msgs_n[1]["id"], "m-204a");
    assert_eq!(msgs_n[2]["id"], "m-204b");
}

#[tokio::test]
async fn scenario_205_blocked_author_in_shown_group_refused_parity() {
    let h = harness().await;
    let (gw, gn) = both_rpc(&h, "group.create", json!({ "name": "Block Group" })).await;
    let group_w = gw["result"]["conversation_id"].as_str().unwrap().to_string();
    let group_n = gn["result"]["conversation_id"].as_str().unwrap().to_string();

    both_rpc(
        &h,
        "block.add",
        json!({ "person_did": "did:key:zBlocked205", "address": "did:key:zBlocked205" }),
    )
    .await;

    h.deliver(true, inbound("m-205", &group_w, "did:key:zBlocked205", 1_000, "spam")).await;
    h.deliver(false, inbound("m-205", &group_n, "did:key:zBlocked205", 1_000, "spam")).await;

    for wasm in [true, false] {
        let refused = h.conv_rows(wasm, "refused_messages").await;
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0]["reason"], "blocked");
    }

    let hw = one_rpc(&h, true, "conversation.history", json!({ "conversation": group_w })).await;
    let hn = one_rpc(&h, false, "conversation.history", json!({ "conversation": group_n })).await;
    assert_eq!(hw["result"]["messages"].as_array().unwrap().len(), 1);
    assert_eq!(hn["result"]["messages"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn scenario_206_group_profile_from_non_owner_and_ordering_parity() {
    let h = harness().await;
    let (gw, gn) = both_rpc(&h, "group.create", json!({ "name": "Initial" })).await;
    let group_w = gw["result"]["conversation_id"].as_str().unwrap().to_string();
    let group_n = gn["result"]["conversation_id"].as_str().unwrap().to_string();
    let conv_svc = did_for_service("conversation");

    let bad_body = group_profile_body("Hijacked");
    h.deliver(
        true,
        inbound_custom(
            "p-bad",
            &group_w,
            "did:key:zNotOwner206",
            1_000,
            GROUP_PROFILE_CONTENT_TYPE,
            bad_body.clone(),
        ),
    )
    .await;
    h.deliver(
        false,
        inbound_custom(
            "p-bad",
            &group_n,
            "did:key:zNotOwner206",
            1_000,
            GROUP_PROFILE_CONTENT_TYPE,
            bad_body,
        ),
    )
    .await;

    for wasm in [true, false] {
        let refused = h.conv_rows(wasm, "refused_messages").await;
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0]["reason"], "not-owner");
    }

    let lw0 = one_rpc(&h, true, "conversation.list", json!({ "kind": "group" })).await;
    let init_ts = lw0["result"]["conversations"][0]["group"]["name_source"]["sender_timestamp_ms"]
        .as_i64()
        .unwrap();
    let ts_newer = init_ts + 2_000;
    let ts_older = init_ts + 1_000;

    let good_body_newer = group_profile_body("New Name");
    h.deliver(
        true,
        inbound_custom(
            "p-newer",
            &group_w,
            &conv_svc,
            ts_newer,
            GROUP_PROFILE_CONTENT_TYPE,
            good_body_newer.clone(),
        ),
    )
    .await;
    h.deliver(
        false,
        inbound_custom(
            "p-newer",
            &group_n,
            &conv_svc,
            ts_newer,
            GROUP_PROFILE_CONTENT_TYPE,
            good_body_newer,
        ),
    )
    .await;

    let lw = one_rpc(&h, true, "conversation.list", json!({ "kind": "group" })).await;
    let ln = one_rpc(&h, false, "conversation.list", json!({ "kind": "group" })).await;
    assert_eq!(lw["result"]["conversations"][0]["group"]["name"], "New Name");
    assert_eq!(ln["result"]["conversations"][0]["group"]["name"], "New Name");

    let stale_body_older = group_profile_body("Stale Name");
    h.deliver(
        true,
        inbound_custom(
            "p-older",
            &group_w,
            &conv_svc,
            ts_older,
            GROUP_PROFILE_CONTENT_TYPE,
            stale_body_older.clone(),
        ),
    )
    .await;
    h.deliver(
        false,
        inbound_custom(
            "p-older",
            &group_n,
            &conv_svc,
            ts_older,
            GROUP_PROFILE_CONTENT_TYPE,
            stale_body_older,
        ),
    )
    .await;

    let lw2 = one_rpc(&h, true, "conversation.list", json!({ "kind": "group" })).await;
    let ln2 = one_rpc(&h, false, "conversation.list", json!({ "kind": "group" })).await;
    assert_eq!(lw2["result"]["conversations"][0]["group"]["name"], "New Name");
    assert_eq!(ln2["result"]["conversations"][0]["group"]["name"], "New Name");
}

#[tokio::test]
async fn scenario_207_cards_refused_in_groups_parity() {
    let h = harness().await;
    let (gw, gn) = both_rpc(&h, "group.create", json!({ "name": "No Cards" })).await;
    let group_w = gw["result"]["conversation_id"].as_str().unwrap().to_string();
    let group_n = gn["result"]["conversation_id"].as_str().unwrap().to_string();

    let sw = one_rpc(
        &h,
        true,
        "conversation.send",
        json!({
            "conversation": group_w,
            "content_type": CARD_CONTENT_TYPE,
            "body": "{}",
        }),
    )
    .await;
    let sn = one_rpc(
        &h,
        false,
        "conversation.send",
        json!({
            "conversation": group_n,
            "content_type": CARD_CONTENT_TYPE,
            "body": "{}",
        }),
    )
    .await;
    assert_eq!(sw["error"]["message"], CARDS_NOT_IN_GROUPS_MESSAGE);
    assert_eq!(sn["error"]["message"], CARDS_NOT_IN_GROUPS_MESSAGE);

    let sync_w = one_rpc(&h, true, "transaction.sync", json!({ "conversation": group_w })).await;
    let sync_n = one_rpc(&h, false, "transaction.sync", json!({ "conversation": group_n })).await;
    assert_eq!(sync_w["error"]["message"], CARDS_NOT_IN_GROUPS_MESSAGE);
    assert_eq!(sync_n["error"]["message"], CARDS_NOT_IN_GROUPS_MESSAGE);
}

#[tokio::test]
async fn scenario_208_delete_incoming_group_message_and_protect_system_rows_parity() {
    let h = harness().await;
    let (gw, gn) = both_rpc(&h, "group.create", json!({ "name": "Delete Test" })).await;
    let group_w = gw["result"]["conversation_id"].as_str().unwrap().to_string();
    let group_n = gn["result"]["conversation_id"].as_str().unwrap().to_string();

    h.deliver(true, inbound("m-208", &group_w, "did:key:zPeer208", 1_000, "delete me")).await;
    h.deliver(false, inbound("m-208", &group_n, "did:key:zPeer208", 1_000, "delete me")).await;

    let hw = one_rpc(&h, true, "conversation.history", json!({ "conversation": group_w })).await;
    let hn = one_rpc(&h, false, "conversation.history", json!({ "conversation": group_n })).await;
    let genesis_id_w = hw["result"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["content_type"] == MEMBERSHIP_EVENT_CONTENT_TYPE)
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let genesis_id_n = hn["result"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["content_type"] == MEMBERSHIP_EVENT_CONTENT_TYPE)
        .unwrap()["id"]
        .as_str()
        .unwrap();

    let dw_sys =
        one_rpc(&h, true, "conversation.delete-message", json!({ "message_id": genesis_id_w }))
            .await;
    let dn_sys =
        one_rpc(&h, false, "conversation.delete-message", json!({ "message_id": genesis_id_n }))
            .await;
    assert_eq!(dw_sys["error"]["message"], "this row records a group change and cannot be deleted");
    assert_eq!(dn_sys["error"]["message"], "this row records a group change and cannot be deleted");

    let dw_msg =
        one_rpc(&h, true, "conversation.delete-message", json!({ "message_id": "m-208" })).await;
    let dn_msg =
        one_rpc(&h, false, "conversation.delete-message", json!({ "message_id": "m-208" })).await;
    assert_eq!(dw_msg["result"]["asked_peer"], false);
    assert_eq!(dn_msg["result"]["asked_peer"], false);
    assert_eq!(dw_msg["result"]["note"], dn_msg["result"]["note"]);
    assert!(dw_msg["result"]["note"].as_str().unwrap().contains("message you received"));

    verify_deletion_request_refusal(&h, &group_w, &group_n, genesis_id_w, genesis_id_n).await;
}

async fn verify_deletion_request_refusal(
    h: &Harness,
    group_w: &str,
    group_n: &str,
    genesis_id_w: &str,
    genesis_id_n: &str,
) {
    let conv_svc = did_for_service("conversation");
    h.deliver(
        true,
        inbound_custom(
            "del-req-w",
            group_w,
            &conv_svc,
            10_000,
            DELETION_REQUEST_CONTENT_TYPE,
            deletion_request_body(genesis_id_w),
        ),
    )
    .await;
    h.deliver(
        false,
        inbound_custom(
            "del-req-n",
            group_n,
            &conv_svc,
            10_000,
            DELETION_REQUEST_CONTENT_TYPE,
            deletion_request_body(genesis_id_n),
        ),
    )
    .await;

    let hw2 = one_rpc(h, true, "conversation.history", json!({ "conversation": group_w })).await;
    let hn2 = one_rpc(h, false, "conversation.history", json!({ "conversation": group_n })).await;
    let gen_w_after = hw2["result"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == genesis_id_w)
        .unwrap();
    let gen_n_after = hn2["result"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == genesis_id_n)
        .unwrap();
    assert!(gen_w_after["deleted_at_secs"].is_null());
    assert!(gen_n_after["deleted_at_secs"].is_null());

    let sw_del = one_rpc(
        h,
        true,
        "conversation.send",
        json!({
            "conversation": group_w,
            "body": "{}",
            "content_type": DELETION_REQUEST_CONTENT_TYPE,
        }),
    )
    .await;
    let sn_del = one_rpc(
        h,
        false,
        "conversation.send",
        json!({
            "conversation": group_n,
            "body": "{}",
            "content_type": DELETION_REQUEST_CONTENT_TYPE,
        }),
    )
    .await;
    assert_eq!(sw_del["error"]["message"], "this content type is reserved");
    assert_eq!(sn_del["error"]["message"], "this content type is reserved");
}

#[tokio::test]
async fn scenario_209_transcript_digest_parity() {
    let h = harness().await;
    let (gw, gn) = both_rpc(&h, "group.create", json!({ "name": "Digest Test" })).await;
    let group_w = gw["result"]["conversation_id"].as_str().unwrap().to_string();
    let group_n = gn["result"]["conversation_id"].as_str().unwrap().to_string();

    let dw1 =
        one_rpc(&h, true, "conversation.transcript-digest", json!({ "conversation": group_w }))
            .await;
    let dw2 =
        one_rpc(&h, true, "conversation.transcript-digest", json!({ "conversation": group_w }))
            .await;
    assert_eq!(dw1, dw2);
    assert_eq!(dw1["result"]["rows"], 1);

    let dn1 =
        one_rpc(&h, false, "conversation.transcript-digest", json!({ "conversation": group_n }))
            .await;
    let dn2 =
        one_rpc(&h, false, "conversation.transcript-digest", json!({ "conversation": group_n }))
            .await;
    assert_eq!(dn1, dn2);
    assert_eq!(dn1["result"]["rows"], 1);

    h.deliver(true, inbound("m-209", &group_w, "did:key:zPeer209", 1_000, "digest msg")).await;
    h.deliver(false, inbound("m-209", &group_n, "did:key:zPeer209", 1_000, "digest msg")).await;

    let dw3 =
        one_rpc(&h, true, "conversation.transcript-digest", json!({ "conversation": group_w }))
            .await;
    let dn3 =
        one_rpc(&h, false, "conversation.transcript-digest", json!({ "conversation": group_n }))
            .await;
    assert_ne!(dw1["result"]["digest"], dw3["result"]["digest"]);
    assert_ne!(dn1["result"]["digest"], dn3["result"]["digest"]);
    assert_eq!(dw3["result"]["rows"], 2);
    assert_eq!(dn3["result"]["rows"], 2);

    one_rpc(&h, true, "conversation.delete-message", json!({ "message_id": "m-209" })).await;
    one_rpc(&h, false, "conversation.delete-message", json!({ "message_id": "m-209" })).await;
    let dw4 =
        one_rpc(&h, true, "conversation.transcript-digest", json!({ "conversation": group_w }))
            .await;
    let dn4 =
        one_rpc(&h, false, "conversation.transcript-digest", json!({ "conversation": group_n }))
            .await;
    assert_eq!(dw3["result"]["digest"], dw4["result"]["digest"]);
    assert_eq!(dn3["result"]["digest"], dn4["result"]["digest"]);
    assert_eq!(dw4["result"]["rows"], 2);
    assert_eq!(dn4["result"]["rows"], 2);
}
