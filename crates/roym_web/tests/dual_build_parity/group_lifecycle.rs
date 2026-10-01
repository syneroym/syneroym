use serde_json::json;
use syneroym_data_db::host_store::RecordWriteValue;
use syneroym_roym_core::{
    card::CARD_CONTENT_TYPE,
    conversation::group::{
        CARDS_NOT_IN_GROUPS_MESSAGE, GROUP_PROFILE_CONTENT_TYPE, GROUP_RESTORED_NOTICE,
        group_profile_body,
    },
};
use syneroym_rpc::{ConversationDeliveryState, ConversationHost, ConversationMessage};

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
async fn scenario_210_hide_and_unhide_parity() {
    let h = harness().await;
    let (gw, gn) = both_rpc(&h, "group.create", json!({ "name": "Hide Test" })).await;
    let group_w = gw["result"]["conversation_id"].as_str().unwrap().to_string();
    let group_n = gn["result"]["conversation_id"].as_str().unwrap().to_string();

    let hw = one_rpc(&h, true, "group.hide", json!({ "conversation": group_w })).await;
    let hn = one_rpc(&h, false, "group.hide", json!({ "conversation": group_n })).await;
    assert_eq!(hw["result"]["admission"]["state"], "hidden");
    assert_eq!(hn["result"]["admission"]["state"], "hidden");

    h.deliver(true, inbound("m-210a", &group_w, "did:key:zPeer210", 1_000, "hidden msg")).await;
    h.deliver(false, inbound("m-210a", &group_n, "did:key:zPeer210", 1_000, "hidden msg")).await;

    for wasm in [true, false] {
        let refused = h.conv_rows(wasm, "refused_messages").await;
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0]["reason"], "group-hidden");
    }

    let lw = one_rpc(&h, true, "conversation.list", json!({ "kind": "group" })).await;
    let ln = one_rpc(&h, false, "conversation.list", json!({ "kind": "group" })).await;
    assert_eq!(lw["result"]["conversations"].as_array().unwrap().len(), 0);
    assert_eq!(ln["result"]["conversations"].as_array().unwrap().len(), 0);
    let lw_inc =
        one_rpc(&h, true, "conversation.list", json!({ "kind": "group", "include_hidden": true }))
            .await;
    let ln_inc =
        one_rpc(&h, false, "conversation.list", json!({ "kind": "group", "include_hidden": true }))
            .await;
    assert_eq!(lw_inc["result"]["conversations"].as_array().unwrap().len(), 1);
    assert_eq!(ln_inc["result"]["conversations"].as_array().unwrap().len(), 1);

    let unw = one_rpc(&h, true, "group.unhide", json!({ "conversation": group_w })).await;
    let unn = one_rpc(&h, false, "group.unhide", json!({ "conversation": group_n })).await;
    assert_eq!(unw["result"]["admission"]["state"], "shown");
    assert_eq!(unn["result"]["admission"]["state"], "shown");
    assert_eq!(unw["result"]["filled_in"], 0);
    assert_eq!(unn["result"]["filled_in"], 0);

    h.deliver(true, inbound("m-210b", &group_w, "did:key:zPeer210", 2_000, "shown msg")).await;
    h.deliver(false, inbound("m-210b", &group_n, "did:key:zPeer210", 2_000, "shown msg")).await;

    let hist_w =
        one_rpc(&h, true, "conversation.history", json!({ "conversation": group_w })).await;
    let hist_n =
        one_rpc(&h, false, "conversation.history", json!({ "conversation": group_n })).await;
    assert_eq!(hist_w["result"]["messages"].as_array().unwrap().len(), 2);
    assert_eq!(hist_n["result"]["messages"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn scenario_211_search_kind_and_system_type_filtering_parity() {
    let h = harness().await;
    let (gw, gn) = both_rpc(&h, "group.create", json!({ "name": "Apples Group" })).await;
    let group_w = gw["result"]["conversation_id"].as_str().unwrap().to_string();
    let group_n = gn["result"]["conversation_id"].as_str().unwrap().to_string();

    let direct_w = open_conv(&h, "did:key:zDirectPeer211").await;
    let direct_n = direct_w.clone();

    h.deliver(true, inbound("m-grp", &group_w, "did:key:zPeerGrp", 1_000, "apples and group"))
        .await;
    h.deliver(false, inbound("m-grp", &group_n, "did:key:zPeerGrp", 1_000, "apples and group"))
        .await;
    h.deliver(true, inbound("m-dir", &direct_w, "did:key:zDirectPeer211", 2_000, "apples direct"))
        .await;
    h.deliver(false, inbound("m-dir", &direct_n, "did:key:zDirectPeer211", 2_000, "apples direct"))
        .await;

    let sw_dir =
        one_rpc(&h, true, "conversation.search", json!({ "query": "apples", "kind": "direct" }))
            .await;
    let sn_dir =
        one_rpc(&h, false, "conversation.search", json!({ "query": "apples", "kind": "direct" }))
            .await;
    assert_eq!(sw_dir["result"]["matches"].as_array().unwrap().len(), 1);
    assert_eq!(sn_dir["result"]["matches"].as_array().unwrap().len(), 1);
    assert!(sw_dir["result"]["matches"][0]["body"].as_str().unwrap().contains("apples direct"));

    let sw_grp =
        one_rpc(&h, true, "conversation.search", json!({ "query": "apples", "kind": "group" }))
            .await;
    let sn_grp =
        one_rpc(&h, false, "conversation.search", json!({ "query": "apples", "kind": "group" }))
            .await;
    assert_eq!(sw_grp["result"]["matches"].as_array().unwrap().len(), 1);
    assert_eq!(sn_grp["result"]["matches"].as_array().unwrap().len(), 1);
    assert!(sw_grp["result"]["matches"][0]["body"].as_str().unwrap().contains("apples and group"));

    let conv_svc = did_for_service("conversation");
    let club_body = group_profile_body("Apples Club");
    h.deliver(
        true,
        inbound_custom(
            "p-club-w",
            &group_w,
            &conv_svc,
            50_000,
            GROUP_PROFILE_CONTENT_TYPE,
            club_body.clone(),
        ),
    )
    .await;
    h.deliver(
        false,
        inbound_custom(
            "p-club-n",
            &group_n,
            &conv_svc,
            50_000,
            GROUP_PROFILE_CONTENT_TYPE,
            club_body,
        ),
    )
    .await;

    let sw_prof = one_rpc(&h, true, "conversation.search", json!({ "query": "Club" })).await;
    let sn_prof = one_rpc(&h, false, "conversation.search", json!({ "query": "Club" })).await;
    assert_eq!(sw_prof["result"]["matches"].as_array().unwrap().len(), 0);
    assert_eq!(sn_prof["result"]["matches"].as_array().unwrap().len(), 0);

    let sw_sys = one_rpc(&h, true, "conversation.search", json!({ "query": "action" })).await;
    let sn_sys = one_rpc(&h, false, "conversation.search", json!({ "query": "action" })).await;
    assert_eq!(sw_sys["result"]["matches"].as_array().unwrap().len(), 0);
    assert_eq!(sn_sys["result"]["matches"].as_array().unwrap().len(), 0);

    assert_eq!(sw_grp["result"]["matches"][0]["author"], sn_grp["result"]["matches"][0]["author"]);
    assert_eq!(sw_grp["result"]["matches"][0]["body"], sn_grp["result"]["matches"][0]["body"]);
    assert_eq!(
        sw_grp["result"]["matches"][0]["content_type"],
        sn_grp["result"]["matches"][0]["content_type"]
    );

    let mut sw_copy = sw_dir;
    let mut sn_copy = sn_dir;
    strip_volatile(&mut sw_copy);
    strip_volatile(&mut sn_copy);
    if let (Some(w_matches), Some(n_matches)) =
        (sw_copy["result"]["matches"].as_array_mut(), sn_copy["result"]["matches"].as_array_mut())
    {
        for m in w_matches.iter_mut().chain(n_matches.iter_mut()) {
            if let Some(obj) = m.as_object_mut() {
                obj.remove("id");
            }
        }
    }
    assert_eq!(sw_copy, sn_copy);
}

#[tokio::test]
async fn scenario_212_export_import_roundtrip_restored_only_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    let (gw, gn) = both_rpc(&h, "group.create", json!({ "name": "Restore Me" })).await;
    let group_w = gw["result"]["conversation_id"].as_str().unwrap().to_string();
    let group_n = gn["result"]["conversation_id"].as_str().unwrap().to_string();

    let (exp_w, exp_n) = both_rpc(&h, "conversation.export", json!({})).await;
    assert_eq!(exp_w["result"]["manifest"]["sections"]["conversations"]["schema_version"], 3);
    assert_eq!(exp_n["result"]["manifest"]["sections"]["conversations"]["schema_version"], 3);

    let h2 = harness().await;
    one_rpc(&h2, true, "conversation.import", json!({ "bundle": exp_w["result"].clone() })).await;
    one_rpc(&h2, false, "conversation.import", json!({ "bundle": exp_n["result"].clone() })).await;

    let iw = one_rpc(&h2, true, "group.info", json!({ "conversation": group_w })).await;
    let in_ = one_rpc(&h2, false, "group.info", json!({ "conversation": group_n })).await;
    assert_eq!(iw["result"]["restored_only"], true);
    assert_eq!(in_["result"]["restored_only"], true);
    assert_eq!(iw["result"]["name"], "Restore Me");
    assert_eq!(in_["result"]["name"], "Restore Me");
    assert_eq!(iw["result"]["can_read_new_messages"], false);
    assert_eq!(in_["result"]["can_read_new_messages"], false);
    assert_eq!(iw["result"]["notices"]["restored"], GROUP_RESTORED_NOTICE);
    assert_eq!(in_["result"]["notices"]["restored"], GROUP_RESTORED_NOTICE);

    let hw = one_rpc(&h2, true, "conversation.history", json!({ "conversation": group_w })).await;
    let hn = one_rpc(&h2, false, "conversation.history", json!({ "conversation": group_n })).await;
    let msgs_w = hw["result"]["messages"].as_array().unwrap();
    let msgs_n = hn["result"]["messages"].as_array().unwrap();
    assert_eq!(msgs_w.len(), 1);
    assert_eq!(msgs_n.len(), 1);

    let genesis_id_w = msgs_w[0]["id"].as_str().unwrap();
    let genesis_id_n = msgs_n[0]["id"].as_str().unwrap();
    verify_restored_group_deletions(&h2, &group_w, &group_n, genesis_id_w, genesis_id_n).await;

    let send_res_w = one_rpc(
        &h2,
        true,
        "conversation.send",
        json!({ "conversation": group_w, "body": "hello" }),
    )
    .await;
    let send_res_n = one_rpc(
        &h2,
        false,
        "conversation.send",
        json!({ "conversation": group_n, "body": "hello" }),
    )
    .await;
    assert_eq!(send_res_w["error"]["message"], GROUP_RESTORED_NOTICE);
    assert_eq!(send_res_n["error"]["message"], GROUP_RESTORED_NOTICE);
}

async fn seed_outgoing_message(h: &Harness, wasm: bool, msg_id: &str, conv_id: &str) {
    let (storage, ks) =
        if wasm { (&h.wasm_storage, &h.wasm_ks) } else { (&h.native_storage, &h.native_ks) };
    let db = storage
        .open_service_db(&did_for_service("conversation"), ks)
        .await
        .expect("open conversation db");
    let row = json!({
        "id": msg_id,
        "conversation": conv_id,
        "author": owner_did(),
        "direction": "outgoing",
        "sender_timestamp_ms": 25_000,
        "content_type": "text/plain",
        "body_encoding": "utf8",
        "body": "test outgoing message",
        "state": "delivered",
        "stored_at_secs": 25,
    });
    let bytes = serde_json::to_vec(&row).expect("serialize outgoing message");
    let write_val = RecordWriteValue { id: msg_id.to_string(), payload: bytes };
    db.put("messages", &write_val, "seed", None).await.expect("put outgoing message");
}

async fn verify_restored_group_deletions(
    h2: &Harness,
    group_w: &str,
    group_n: &str,
    genesis_id_w: &str,
    genesis_id_n: &str,
) {
    let del_restored_w =
        one_rpc(h2, true, "conversation.delete-message", json!({ "message_id": genesis_id_w }))
            .await;
    let del_restored_n =
        one_rpc(h2, false, "conversation.delete-message", json!({ "message_id": genesis_id_n }))
            .await;
    assert_eq!(
        del_restored_w["error"]["message"],
        "this row records a group change and cannot be deleted"
    );
    assert_eq!(
        del_restored_n["error"]["message"],
        "this row records a group change and cannot be deleted"
    );

    let conv_svc = did_for_service("conversation");
    h2.deliver(
        true,
        inbound_custom("inj-w", group_w, &conv_svc, 20_000, "text/plain", b"hi".to_vec()),
    )
    .await;
    h2.deliver(
        false,
        inbound_custom("inj-n", group_n, &conv_svc, 20_000, "text/plain", b"hi".to_vec()),
    )
    .await;

    let del_inj_w = one_rpc(
        h2,
        true,
        "conversation.delete-message",
        json!({ "message_id": "inj-w", "ask_peer": true }),
    )
    .await;
    let del_inj_n = one_rpc(
        h2,
        false,
        "conversation.delete-message",
        json!({ "message_id": "inj-n", "ask_peer": true }),
    )
    .await;
    assert!(del_inj_w["error"].is_null());
    assert_eq!(del_inj_w["result"]["asked_peer"], false);
    assert_eq!(del_inj_w["result"]["deleted"], "inj-w");
    assert!(del_inj_w["result"]["note"].as_str().unwrap().contains("message you received"));
    assert!(del_inj_n["error"].is_null());
    assert_eq!(del_inj_n["result"]["asked_peer"], false);
    assert_eq!(del_inj_n["result"]["deleted"], "inj-n");
    assert_eq!(del_inj_w["result"]["note"], del_inj_n["result"]["note"]);

    seed_outgoing_message(h2, true, "out-w", group_w).await;
    seed_outgoing_message(h2, false, "out-n", group_n).await;

    let del_out_w = one_rpc(
        h2,
        true,
        "conversation.delete-message",
        json!({ "message_id": "out-w", "ask_peer": true }),
    )
    .await;
    let del_out_n = one_rpc(
        h2,
        false,
        "conversation.delete-message",
        json!({ "message_id": "out-n", "ask_peer": true }),
    )
    .await;
    assert!(del_out_w["error"].is_null());
    assert_eq!(del_out_w["result"]["asked_peer"], false);
    assert_eq!(del_out_w["result"]["deleted"], "out-w");
    assert!(
        del_out_w["result"]["note"]
            .as_str()
            .unwrap()
            .contains("nobody else in this group can receive one from you now")
    );
    assert!(del_out_n["error"].is_null());
    assert_eq!(del_out_n["result"]["asked_peer"], false);
    assert_eq!(del_out_n["result"]["deleted"], "out-n");
    assert_eq!(del_out_w["result"]["note"], del_out_n["result"]["note"]);
}

#[tokio::test]
async fn scenario_213_adoption_without_a_message_parity() {
    let h = harness().await;
    let conv_svc = did_for_service("conversation");
    let _gw = h.wasm_conversation.create_group(&conv_svc).await.unwrap();
    let _gn = h.native_conversation.create_group(&conv_svc).await.unwrap();

    let lw_grp = one_rpc(&h, true, "conversation.list", json!({ "kind": "group" })).await;
    let ln_grp = one_rpc(&h, false, "conversation.list", json!({ "kind": "group" })).await;
    assert_eq!(lw_grp["result"]["conversations"].as_array().unwrap().len(), 1);
    assert_eq!(ln_grp["result"]["conversations"].as_array().unwrap().len(), 1);
    assert_eq!(lw_grp["result"]["conversations"][0]["kind"], "group");
    assert_eq!(ln_grp["result"]["conversations"][0]["kind"], "group");

    let lw_dir = one_rpc(&h, true, "conversation.list", json!({ "kind": "direct" })).await;
    let ln_dir = one_rpc(&h, false, "conversation.list", json!({ "kind": "direct" })).await;
    assert_eq!(lw_dir["result"]["conversations"].as_array().unwrap().len(), 0);
    assert_eq!(ln_dir["result"]["conversations"].as_array().unwrap().len(), 0);

    let gw_no_row = h.wasm_conversation.create_group(&conv_svc).await.unwrap();
    let gn_no_row = h.native_conversation.create_group(&conv_svc).await.unwrap();
    let sw_card = one_rpc(
        &h,
        true,
        "conversation.send",
        json!({
            "conversation": gw_no_row,
            "body": "{}",
            "content_type": CARD_CONTENT_TYPE,
        }),
    )
    .await;
    let sn_card = one_rpc(
        &h,
        false,
        "conversation.send",
        json!({
            "conversation": gn_no_row,
            "body": "{}",
            "content_type": CARD_CONTENT_TYPE,
        }),
    )
    .await;
    assert_eq!(sw_card["error"]["message"], CARDS_NOT_IN_GROUPS_MESSAGE);
    assert_eq!(sn_card["error"]["message"], CARDS_NOT_IN_GROUPS_MESSAGE);
}

#[tokio::test]
async fn scenario_214_sent_row_carries_host_values_parity() {
    let h = harness().await;
    let conv_svc = did_for_service("conversation");
    let conv = open_conv(&h, "did:key:zNeverAnswering214").await;

    let (sw, sn) =
        both_rpc(&h, "conversation.send", json!({ "conversation": conv, "body": "test readback" }))
            .await;
    let msg_id_w = sw["result"]["message_id"].as_str().unwrap();
    let msg_id_n = sn["result"]["message_id"].as_str().unwrap();

    let host_msg_w = h.wasm_conversation.get_message(&conv_svc, msg_id_w).await.unwrap();
    let host_msg_n = h.native_conversation.get_message(&conv_svc, msg_id_n).await.unwrap();

    let hw = one_rpc(&h, true, "conversation.history", json!({ "conversation": conv })).await;
    let hn = one_rpc(&h, false, "conversation.history", json!({ "conversation": conv })).await;
    let row_w = &hw["result"]["messages"][0];
    let row_n = &hn["result"]["messages"][0];

    assert_eq!(row_w["author"], host_msg_w.author);
    assert_eq!(row_n["author"], host_msg_n.author);
    assert_eq!(row_w["sender_timestamp_ms"], host_msg_w.sender_timestamp);
    assert_eq!(row_n["sender_timestamp_ms"], host_msg_n.sender_timestamp);
}

#[tokio::test]
async fn scenario_215_group_ops_on_direct_conversation_parity() {
    let h = harness().await;
    let conv = open_conv(&h, "did:key:zDirectPeer215").await;

    for verb in ["group.hide", "group.unhide", "group.sync"] {
        let (rw, rn) = both_rpc(&h, verb, json!({ "group": conv })).await;
        assert_eq!(rw["error"]["message"], "not a group conversation");
        assert_eq!(rn["error"]["message"], "not a group conversation");
    }
}
