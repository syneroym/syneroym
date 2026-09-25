use serde_json::json;
use syneroym_rpc::{ConversationDeliveryState, ConversationHost, ConversationMessage};

use super::{fixtures::*, helpers::*};

#[tokio::test]
async fn scenario_52_conversation_open_by_address_parity() {
    let h = harness().await;
    let conv_id = open_conv(&h, "did:key:zPeer52").await;
    assert!(!conv_id.is_empty());

    let (w, n) = both_rpc(&h, "conversation.list", json!({})).await;
    let mut w2 = w.clone();
    let mut n2 = n.clone();
    strip_volatile(&mut w2);
    strip_volatile(&mut n2);
    assert_eq!(w2, n2);
    assert_eq!(w["result"]["conversations"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn scenario_53_conversation_send_is_never_optimistic_parity() {
    let h = harness().await;
    let conv_id = open_conv(&h, "did:key:zPeer53").await;

    let (sw, sn) = both_rpc(
        &h,
        "conversation.send",
        json!({ "conversation": conv_id, "body": "hello there" }),
    )
    .await;
    assert_eq!(sw["result"]["state"], "pending");
    assert_eq!(sn["result"]["state"], "pending");

    let (mut hw, mut hn) =
        both_rpc(&h, "conversation.history", json!({ "conversation": conv_id })).await;
    strip_volatile(&mut hw);
    strip_volatile(&mut hn);
    let cw = normalize_message_ids(&mut hw);
    let cn = normalize_message_ids(&mut hn);
    assert_eq!(cw, 1);
    assert_eq!(cn, 1);
    assert_eq!(hw, hn);
    assert_eq!(hw["result"]["messages"][0]["state"], "pending");
}

#[tokio::test]
async fn scenario_54_inbound_message_reaches_history_parity() {
    let h = harness().await;
    let conv = "conv-54";
    h.deliver(true, inbound("m-54", conv, "did:key:zPeer54", 1_000, "incoming hi")).await;
    h.deliver(false, inbound("m-54", conv, "did:key:zPeer54", 1_000, "incoming hi")).await;

    let (mut hw, mut hn) =
        both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    strip_volatile(&mut hw);
    strip_volatile(&mut hn);
    assert_eq!(normalize_message_ids(&mut hw), 1);
    assert_eq!(normalize_message_ids(&mut hn), 1);
    assert_eq!(hw, hn);
    assert_eq!(hw["result"]["messages"][0]["body"], "incoming hi");
    assert_eq!(hw["result"]["messages"][0]["direction"], "incoming");
}

#[tokio::test]
async fn scenario_55_inbound_order_is_the_rule_not_arrival_parity() {
    let h = harness().await;
    let conv = "conv-55";
    // wasm learns A then B; native learns B then A.
    h.deliver(true, inbound("m-a", conv, "did:key:zPeer55", 1_000, "first")).await;
    h.deliver(true, inbound("m-b", conv, "did:key:zPeer55", 2_000, "second")).await;
    h.deliver(false, inbound("m-b", conv, "did:key:zPeer55", 2_000, "second")).await;
    h.deliver(false, inbound("m-a", conv, "did:key:zPeer55", 1_000, "first")).await;

    let (mut hw, mut hn) =
        both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    strip_volatile(&mut hw);
    strip_volatile(&mut hn);
    assert_eq!(normalize_message_ids(&mut hw), 2);
    assert_eq!(normalize_message_ids(&mut hn), 2);
    assert_eq!(hw, hn);
    let msgs = hw["result"]["messages"].as_array().unwrap();
    assert_eq!(msgs[0]["body"], "first");
    assert_eq!(msgs[1]["body"], "second");
}

#[tokio::test]
async fn scenario_56_delivery_state_failed_parity() {
    let h = harness().await;
    let conv_id = open_conv(&h, "did:key:zPeer56").await;
    let (sw, sn) =
        both_rpc(&h, "conversation.send", json!({ "conversation": conv_id, "body": "will fail" }))
            .await;
    let mw = sw["result"]["message_id"].as_str().unwrap().to_string();
    let mn = sn["result"]["message_id"].as_str().unwrap().to_string();

    h.notify_state(true, &mw, ConversationDeliveryState::Failed).await;
    h.notify_state(false, &mn, ConversationDeliveryState::Failed).await;

    let (mut hw, mut hn) =
        both_rpc(&h, "conversation.history", json!({ "conversation": conv_id })).await;
    strip_volatile(&mut hw);
    strip_volatile(&mut hn);
    assert_eq!(normalize_message_ids(&mut hw), 1);
    assert_eq!(normalize_message_ids(&mut hn), 1);
    assert_eq!(hw, hn);
    assert_eq!(hw["result"]["messages"][0]["state"], "failed");

    // Retry is reached (not method-not-found, not wire-refused) on both.
    let rw = one_rpc(&h, true, "conversation.retry", json!({ "message_id": mw })).await;
    let rn = one_rpc(&h, false, "conversation.retry", json!({ "message_id": mn })).await;
    for r in [&rw, &rn] {
        assert_ne!(r["error"]["code"].as_i64(), Some(-32601));
        assert_ne!(r["error"]["code"].as_i64(), Some(-32013));
    }
}

#[tokio::test]
async fn scenario_57_blocked_sender_never_reaches_inbox_parity() {
    let h = harness().await;
    both_rpc(
        &h,
        "block.add",
        json!({ "person_did": "did:key:zBlocked57", "address": "did:key:zBlocked57" }),
    )
    .await;
    let conv = "conv-57";
    h.deliver(true, inbound("m-57", conv, "did:key:zBlocked57", 1_000, "let me in")).await;
    h.deliver(false, inbound("m-57", conv, "did:key:zBlocked57", 1_000, "let me in")).await;

    let (hw, hn) = both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    assert_eq!(hw["result"]["messages"].as_array().unwrap().len(), 0);
    assert_eq!(hn["result"]["messages"].as_array().unwrap().len(), 0);

    let (lw, ln) = both_rpc(&h, "conversation.list", json!({})).await;
    assert_eq!(lw["result"]["conversations"].as_array().unwrap().len(), 0);
    assert_eq!(ln["result"]["conversations"].as_array().unwrap().len(), 0);

    // Recorded in the bodiless refused collection on both builds.
    for wasm in [true, false] {
        let refused = h.conv_rows(wasm, "refused_messages").await;
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0]["reason"], "blocked");
        assert!(refused[0].get("body").is_none());
    }
}

#[tokio::test]
async fn scenario_58_refused_message_is_counted_nowhere_parity() {
    let h = harness().await;
    both_rpc(
        &h,
        "block.add",
        json!({ "person_did": "did:key:zBlocked58", "address": "did:key:zBlocked58" }),
    )
    .await;
    let conv = "conv-58";
    h.deliver(true, inbound("m-58", conv, "did:key:zBlocked58", 1_000, "secret word")).await;
    h.deliver(false, inbound("m-58", conv, "did:key:zBlocked58", 1_000, "secret word")).await;

    let (sw, sn) = both_rpc(&h, "conversation.search", json!({ "query": "secret" })).await;
    assert_eq!(sw["result"]["matches"].as_array().unwrap().len(), 0);
    assert_eq!(sn["result"]["matches"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn scenario_59_first_contact_rate_limit_at_inbox_parity() {
    let h = harness().await;
    both_rpc(&h, "contacts.set-limits", json!({ "window_secs": 3600, "max_per_window": 2 })).await;

    let mut admitted_w = 0;
    let mut admitted_n = 0;
    for i in 0..4 {
        let conv = format!("conv-59-{i}");
        h.deliver(true, inbound(&format!("m-59-{i}"), &conv, "did:key:zStranger59", 1_000, "hi"))
            .await;
        h.deliver(false, inbound(&format!("m-59-{i}"), &conv, "did:key:zStranger59", 1_000, "hi"))
            .await;
        let (hw, hn) = both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
        admitted_w += hw["result"]["messages"].as_array().unwrap().len();
        admitted_n += hn["result"]["messages"].as_array().unwrap().len();
    }
    assert_eq!(admitted_w, 2);
    assert_eq!(admitted_n, 2);
}

#[tokio::test]
async fn scenario_60_group_kind_is_refused_as_unsupported_parity() {
    let h = harness().await;
    let conv_svc = did_for_service("conversation");
    let gw = h.wasm_conversation.create_group(&conv_svc).await.unwrap();
    let gn = h.native_conversation.create_group(&conv_svc).await.unwrap();

    h.deliver(true, inbound("m-60", &gw, "did:key:zPeer60", 1_000, "group hi")).await;
    h.deliver(false, inbound("m-60", &gn, "did:key:zPeer60", 1_000, "group hi")).await;

    let (hw, hn) = both_rpc(&h, "conversation.history", json!({ "conversation": gw })).await;
    assert_eq!(hw["result"]["messages"].as_array().unwrap().len(), 0);
    let _ = hn;
    let (lw, _) = both_rpc(&h, "conversation.list", json!({})).await;
    assert_eq!(lw["result"]["conversations"].as_array().unwrap().len(), 0);

    for wasm in [true, false] {
        let refused = h.conv_rows(wasm, "refused_messages").await;
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0]["reason"], "unsupported-kind");
    }
}

#[tokio::test]
async fn scenario_61_delete_outgoing_message_asks_peer_parity() {
    let h = harness().await;
    let conv_id = open_conv(&h, "did:key:zPeer61").await;
    let (sw, sn) =
        both_rpc(&h, "conversation.send", json!({ "conversation": conv_id, "body": "oops" })).await;
    let mw = sw["result"]["message_id"].as_str().unwrap().to_string();
    let mn = sn["result"]["message_id"].as_str().unwrap().to_string();

    // Each stack deletes its own message id; compare the response shape.
    let dwv = one_rpc(&h, true, "conversation.delete-message", json!({ "message_id": mw })).await;
    let dnv = one_rpc(&h, false, "conversation.delete-message", json!({ "message_id": mn })).await;
    assert_eq!(dwv["result"]["asked_peer"], true);
    assert_eq!(dnv["result"]["asked_peer"], true);
    assert_eq!(dwv["result"]["note"], dnv["result"]["note"]);
    assert!(dwv["result"]["note"].as_str().unwrap().contains("other side"));

    // The tombstoned row keeps its place and loses its body.
    let (mut hw, mut hn) =
        both_rpc(&h, "conversation.history", json!({ "conversation": conv_id })).await;
    strip_volatile(&mut hw);
    strip_volatile(&mut hn);
    assert_eq!(normalize_message_ids(&mut hw), 1);
    assert_eq!(normalize_message_ids(&mut hn), 1);
    assert!(hw["result"]["messages"][0].get("body").is_none());
    assert_eq!(hw, hn);

    // One reserved-content-type message queued in the host outbox on both.
    for wasm in [true, false] {
        let (ow, on) = both_rpc(&h, "conversation.outbox", json!({})).await;
        let ob = if wasm { &ow } else { &on };
        let entries = ob["result"]["outbox"].as_array().unwrap();
        assert!(!entries.is_empty(), "a deletion request must be queued");
    }
}

#[tokio::test]
async fn scenario_62_inbound_deletion_request_honoured_only_for_own_message_parity() {
    let h = harness().await;
    let conv = "conv-62";
    // Peer's own message, delivered inbound.
    h.deliver(true, inbound("m-62", conv, "did:key:zPeer62", 1_000, "keep me")).await;
    h.deliver(false, inbound("m-62", conv, "did:key:zPeer62", 1_000, "keep me")).await;

    // A deletion request from the same peer, naming their own message.
    let del_own = |target: &str| ConversationMessage {
        id: format!("del-{target}"),
        conversation: conv.to_string(),
        author: "did:key:zPeer62".to_string(),
        sender_timestamp: 2_000,
        received_at: 2_000,
        content_type: "application/vnd.roym.deletion-request+json".to_string(),
        body: json!({ "message_id": target }).to_string().into_bytes(),
        state: ConversationDeliveryState::Delivered,
        verified: true,
        last_error: None,
    };
    h.deliver(true, del_own("m-62")).await;
    h.deliver(false, del_own("m-62")).await;

    let (hw, hn) = both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    assert!(hw["result"]["messages"][0].get("body").is_none(), "own message must be tombstoned");
    assert!(hn["result"]["messages"][0].get("body").is_none());

    // A deletion request naming a message the requester did NOT author
    // changes nothing.
    h.deliver(true, {
        let mut m = del_own("m-62");
        m.author = "did:key:zSomeoneElse".to_string();
        m.id = "del-other-w".to_string();
        m
    })
    .await;
    let (hw2, _) = both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    assert_eq!(hw2["result"]["messages"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn scenario_63_conversation_export_integrity_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    let conv = "conv-63";
    h.deliver(true, inbound("m-63", conv, "did:key:zPeer63", 1_000, "archive me")).await;
    h.deliver(false, inbound("m-63", conv, "did:key:zPeer63", 1_000, "archive me")).await;

    let (mut w, mut n) = both_rpc(&h, "conversation.export", json!({})).await;
    verify_and_strip_manifest_signature(&mut w);
    verify_and_strip_manifest_signature(&mut n);
    strip_volatile(&mut w);
    strip_volatile(&mut n);
    assert_eq!(normalize_message_ids(&mut w), 1);
    assert_eq!(normalize_message_ids(&mut n), 1);
    assert_eq!(w, n);
}

#[tokio::test]
async fn scenario_64_conversation_import_roundtrip_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    let conv = "conv-64";
    h.deliver(true, inbound("m-64", conv, "did:key:zPeer64", 1_000, "restore me")).await;
    h.deliver(false, inbound("m-64", conv, "did:key:zPeer64", 1_000, "restore me")).await;

    let (exp, _) = both_rpc(&h, "conversation.export", json!({})).await;
    let (impw, impn) =
        both_rpc(&h, "conversation.import", json!({ "bundle": exp["result"].clone() })).await;
    assert!(!is_err(&impw, -32602), "import failed: {impw}");
    assert_eq!(impw["result"]["imported"], impn["result"]["imported"]);

    let (mut hw, mut hn) =
        both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    strip_volatile(&mut hw);
    strip_volatile(&mut hn);
    assert_eq!(normalize_message_ids(&mut hw), 1);
    assert_eq!(normalize_message_ids(&mut hn), 1);
    assert_eq!(hw, hn);
}

#[tokio::test]
async fn scenario_65_conversation_import_tampered_message_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    let conv = "conv-65";
    h.deliver(true, inbound("m-65", conv, "did:key:zPeer65", 1_000, "original")).await;
    h.deliver(false, inbound("m-65", conv, "did:key:zPeer65", 1_000, "original")).await;

    let (exp, _) = both_rpc(&h, "conversation.export", json!({})).await;
    let mut bundle = exp["result"].clone();
    let rows = bundle["sections"]["messages"].as_array_mut().unwrap();
    rows[0]["payload"]["body"] = json!("tampered");

    let (w, n) = both_rpc(&h, "conversation.import", json!({ "bundle": bundle })).await;
    assert_eq!(w, n);
    assert!(is_err(&w, -32602));
}

#[tokio::test]
async fn scenario_66_certificate_verbs_on_catalog_and_conversation_parity() {
    let h = harness().await;
    for service in ["catalog", "conversation"] {
        let (w, n) = both_rpc(&h, &format!("{service}.signing-status"), json!({})).await;
        assert_eq!(w["result"]["certificate"]["state"], "missing");
        assert_eq!(n["result"]["certificate"]["state"], "missing");
        enrol_signing(&h, service).await;
        let (w2, n2) = both_rpc(&h, &format!("{service}.signing-status"), json!({})).await;
        assert_eq!(w2["result"]["certificate"]["state"], "installed");
        assert_eq!(n2["result"]["certificate"]["state"], "installed");
    }
}
