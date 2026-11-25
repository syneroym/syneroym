use serde_json::json;
use syneroym_roym_core::{
    conversation::group,
    widget::{
        WIDGET_ANSWER_CONTENT_TYPE, WIDGET_CONTENT_TYPE, answer_body, poll_definition, widget_body,
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
    let mut m = inbound(id, conversation, author, ts, "");
    m.content_type = content_type.to_string();
    m.body = body;
    m
}

#[tokio::test]
async fn scenario_260_widget_send_and_list_parity() {
    let h = harness().await;
    let conv = open_conv(&h, "did:key:zPeer260").await;
    let poll = poll_definition("Favorite color?", &["Red".into(), "Blue".into()]);

    let (sw, sn) =
        both_rpc(&h, "widget.send", json!({ "conversation": conv, "widget": poll })).await;
    assert_eq!(sw["result"]["state"], "pending");
    assert_eq!(sn["result"]["state"], "pending");

    let wid_w = sw["result"]["message_id"].as_str().unwrap().to_string();
    let wid_n = sn["result"]["message_id"].as_str().unwrap().to_string();

    let mut lw =
        one_rpc(&h, true, "widget.list", json!({ "conversation": conv, "widget_ids": [wid_w] }))
            .await;
    let mut ln =
        one_rpc(&h, false, "widget.list", json!({ "conversation": conv, "widget_ids": [wid_n] }))
            .await;
    strip_volatile(&mut lw);
    strip_volatile(&mut ln);
    normalize_message_ids(&mut lw);
    normalize_message_ids(&mut ln);
    assert_eq!(lw, ln);

    let widget_w = &lw["result"]["widgets"][0];
    assert_eq!(widget_w["checked"], true);
    assert!(widget_w["refused"].is_null());
    assert_eq!(widget_w["answers"], json!([]));
    assert_eq!(widget_w["ignored_answers"], 0);
    assert_eq!(
        widget_w["counts"],
        json!([
            { "field": "option", "choice": "o1", "count": 0 },
            { "field": "option", "choice": "o2", "count": 0 }
        ])
    );

    let (mut lw_all, mut ln_all) =
        both_rpc(&h, "widget.list", json!({ "conversation": conv })).await;
    strip_volatile(&mut lw_all);
    strip_volatile(&mut ln_all);
    normalize_message_ids(&mut lw_all);
    normalize_message_ids(&mut ln_all);
    assert_eq!(lw_all, ln_all);
    assert_eq!(lw_all["result"]["widgets"].as_array().unwrap().len(), 1);
    assert_eq!(lw_all["result"]["complete"], true);
}

async fn deliver_scenario_261_inbound(h: &Harness, conv: &str) {
    let poll = poll_definition("Which path?", &["Left".into(), "Right".into()]);
    let poll_bytes = widget_body(&poll).unwrap().into_bytes();

    let m_poll =
        inbound_custom("w1", conv, "did:key:zAuthor", 1_000, WIDGET_CONTENT_TYPE, poll_bytes);
    h.deliver(true, m_poll.clone()).await;
    h.deliver(false, m_poll).await;

    let ans1 = answer_body("w1", &json!({ "option": "o1" })).unwrap().into_bytes();
    let m_a1 =
        inbound_custom("ans-a1", conv, "did:key:zVoterA", 2_000, WIDGET_ANSWER_CONTENT_TYPE, ans1);
    h.deliver(true, m_a1.clone()).await;
    h.deliver(false, m_a1).await;

    let ans2 = answer_body("w1", &json!({ "option": "o2" })).unwrap().into_bytes();
    let m_a2 =
        inbound_custom("ans-a2", conv, "did:key:zVoterA", 3_000, WIDGET_ANSWER_CONTENT_TYPE, ans2);
    h.deliver(true, m_a2.clone()).await;
    h.deliver(false, m_a2).await;

    let ans_b = answer_body("w1", &json!({ "option": "o1" })).unwrap().into_bytes();
    let m_b1 =
        inbound_custom("ans-b1", conv, "did:key:zVoterB", 2_500, WIDGET_ANSWER_CONTENT_TYPE, ans_b);
    h.deliver(true, m_b1.clone()).await;
    h.deliver(false, m_b1).await;

    let ans_bad = answer_body("w1", &json!({ "option": "o99" })).unwrap().into_bytes();
    let m_b_bad = inbound_custom(
        "ans-b-bad",
        conv,
        "did:key:zVoterB",
        3_500,
        WIDGET_ANSWER_CONTENT_TYPE,
        ans_bad,
    );
    h.deliver(true, m_b_bad.clone()).await;
    h.deliver(false, m_b_bad).await;
}

#[tokio::test]
async fn scenario_261_inbound_answers_count_the_newest_of_each_author_parity() {
    let h = harness().await;
    let conv = open_conv(&h, "did:key:zPeer261").await;
    deliver_scenario_261_inbound(&h, &conv).await;

    let (mut lw, mut ln) =
        both_rpc(&h, "widget.list", json!({ "conversation": conv, "widget_ids": ["w1"] })).await;
    strip_volatile(&mut lw);
    strip_volatile(&mut ln);
    assert_eq!(lw, ln);

    let view = &lw["result"]["widgets"][0];
    assert_eq!(view["ignored_answers"], 1);
    assert_eq!(
        view["counts"],
        json!([
            { "field": "option", "choice": "o1", "count": 1 },
            { "field": "option", "choice": "o2", "count": 1 }
        ])
    );
    assert_eq!(view["answers"].as_array().unwrap().len(), 2);

    both_rpc(
        &h,
        "conversation.delete-message",
        json!({ "message_id": "ans-a2", "ask_peer": false }),
    )
    .await;

    let (mut lw2, mut ln2) =
        both_rpc(&h, "widget.list", json!({ "conversation": conv, "widget_ids": ["w1"] })).await;
    strip_volatile(&mut lw2);
    strip_volatile(&mut ln2);
    assert_eq!(lw2, ln2);

    let view2 = &lw2["result"]["widgets"][0];
    assert_eq!(
        view2["counts"],
        json!([
            { "field": "option", "choice": "o1", "count": 2 },
            { "field": "option", "choice": "o2", "count": 0 }
        ])
    );

    let (ans_own_w, ans_own_n) = both_rpc(
        &h,
        "widget.answer",
        json!({ "conversation": conv, "widget_id": "w1", "data": { "option": "o2" } }),
    )
    .await;
    let own_id_w = ans_own_w["result"]["message_id"].as_str().unwrap().to_string();
    let own_id_n = ans_own_n["result"]["message_id"].as_str().unwrap().to_string();

    h.notify_state(true, &own_id_w, ConversationDeliveryState::Failed).await;
    h.notify_state(false, &own_id_n, ConversationDeliveryState::Failed).await;

    let (mut lw3, mut ln3) =
        both_rpc(&h, "widget.list", json!({ "conversation": conv, "widget_ids": ["w1"] })).await;
    strip_volatile(&mut lw3);
    strip_volatile(&mut ln3);
    normalize_message_ids(&mut lw3);
    normalize_message_ids(&mut ln3);
    assert_eq!(lw3, ln3);

    let view3 = &lw3["result"]["widgets"][0];
    assert_eq!(
        view3["counts"],
        json!([
            { "field": "option", "choice": "o1", "count": 2 },
            { "field": "option", "choice": "o2", "count": 1 }
        ])
    );
    let own_ans =
        view3["answers"].as_array().unwrap().iter().find(|a| a["direction"] == "outgoing").unwrap();
    assert_eq!(own_ans["state"], "failed");
}

async fn assert_widget_send_refusals(h: &Harness, conv: &str) {
    let mut unknown_type_def = poll_definition("Q?", &["A".into(), "B".into()]);
    unknown_type_def.widget_type = "poll/unknown-type".to_string();
    let (w_unk, n_unk) =
        both_rpc(h, "widget.send", json!({ "conversation": conv, "widget": unknown_type_def }))
            .await;
    assert_eq!(w_unk, n_unk);
    assert_eq!(w_unk["error"]["code"], -32602);
    assert_eq!(w_unk["error"]["data"]["code"], "extension-unknown");

    let big_prompt = "x".repeat(8 * 1024);
    let mut large_def = poll_definition("Q?", &["A".into(), "B".into()]);
    large_def.prompt = big_prompt;
    let (w_lg, n_lg) =
        both_rpc(h, "widget.send", json!({ "conversation": conv, "widget": large_def })).await;
    assert_eq!(w_lg, n_lg);
    assert_eq!(w_lg["error"]["data"]["code"], "widget-shape");

    let mut blank_def = poll_definition("   ", &["A".into(), "B".into()]);
    blank_def.prompt = "".to_string();
    let (w_blank, n_blank) =
        both_rpc(h, "widget.send", json!({ "conversation": conv, "widget": blank_def })).await;
    assert_eq!(w_blank, n_blank);
    assert_eq!(w_blank["error"]["data"]["code"], "widget-shape");

    let bad_field_type_input = json!({
        "conversation": conv,
        "widget": {
            "extension": { "id": "poll", "version": 1 },
            "type": "poll/single",
            "semantic": "poll",
            "prompt": "Pick location",
            "fields": [{ "name": "loc", "type": "location" }]
        }
    });
    let (w_bft, n_bft) = both_rpc(h, "widget.send", bad_field_type_input).await;
    assert_eq!(w_bft, n_bft);
    assert_eq!(w_bft["error"]["code"], -32602);

    let (w_res1, n_res1) = both_rpc(
        h,
        "conversation.send",
        json!({ "conversation": conv, "content_type": WIDGET_CONTENT_TYPE, "body": "{}" }),
    )
    .await;
    assert_eq!(w_res1, n_res1);
    assert_eq!(w_res1["error"]["message"], "this content type is reserved");

    let (w_res2, n_res2) = both_rpc(
        h,
        "conversation.send",
        json!({ "conversation": conv, "content_type": WIDGET_ANSWER_CONTENT_TYPE, "body": "{}" }),
    )
    .await;
    assert_eq!(w_res2, n_res2);
    assert_eq!(w_res2["error"]["message"], "this content type is reserved");
}

#[tokio::test]
async fn scenario_262_widget_refusals_parity() {
    let h = harness().await;
    let conv = open_conv(&h, "did:key:zPeer262").await;
    let conv_other = open_conv(&h, "did:key:zPeer262Other").await;

    assert_widget_send_refusals(&h, &conv).await;

    let m_txt = inbound("txt-msg-262", &conv, "did:key:zPeer262", 1_000, "hello");
    h.deliver(true, m_txt.clone()).await;
    h.deliver(false, m_txt).await;

    let (w_not_w, n_not_w) = both_rpc(
        &h,
        "widget.answer",
        json!({ "conversation": conv, "widget_id": "txt-msg-262", "data": { "option": "o1" } }),
    )
    .await;
    assert_eq!(w_not_w, n_not_w);
    assert_eq!(w_not_w["error"]["data"]["code"], "not-a-widget");

    let poll_def = poll_definition("Choice?", &["A".into(), "B".into()]);
    let (w_send, n_send) =
        both_rpc(&h, "widget.send", json!({ "conversation": conv, "widget": poll_def })).await;
    let wid_w = w_send["result"]["message_id"].as_str().unwrap().to_string();
    let wid_n = n_send["result"]["message_id"].as_str().unwrap().to_string();

    let w_foreign = one_rpc(
        &h,
        true,
        "widget.answer",
        json!({ "conversation": conv_other, "widget_id": wid_w, "data": { "option": "o1" } }),
    )
    .await;
    let n_foreign = one_rpc(
        &h,
        false,
        "widget.answer",
        json!({ "conversation": conv_other, "widget_id": wid_n, "data": { "option": "o1" } }),
    )
    .await;
    assert_eq!(w_foreign, n_foreign);
    assert_eq!(w_foreign["error"]["data"]["code"], "widget-not-found");

    let w_ans_bad = one_rpc(
        &h,
        true,
        "widget.answer",
        json!({ "conversation": conv, "widget_id": wid_w, "data": { "option": "o99" } }),
    )
    .await;
    let n_ans_bad = one_rpc(
        &h,
        false,
        "widget.answer",
        json!({ "conversation": conv, "widget_id": wid_n, "data": { "option": "o99" } }),
    )
    .await;
    assert_eq!(w_ans_bad, n_ans_bad);
    assert_eq!(w_ans_bad["error"]["data"]["code"], "answer-invalid");

    one_rpc(
        &h,
        true,
        "conversation.delete-message",
        json!({ "message_id": wid_w, "ask_peer": false }),
    )
    .await;
    one_rpc(
        &h,
        false,
        "conversation.delete-message",
        json!({ "message_id": wid_n, "ask_peer": false }),
    )
    .await;

    let w_del = one_rpc(
        &h,
        true,
        "widget.answer",
        json!({ "conversation": conv, "widget_id": wid_w, "data": { "option": "o1" } }),
    )
    .await;
    let n_del = one_rpc(
        &h,
        false,
        "widget.answer",
        json!({ "conversation": conv, "widget_id": wid_n, "data": { "option": "o1" } }),
    )
    .await;
    assert_eq!(w_del, n_del);
    assert_eq!(w_del["error"]["data"]["code"], "widget-deleted");
}

#[tokio::test]
async fn scenario_263_a_poll_in_a_group_parity() {
    let h = harness().await;
    let (gw, gn) = both_rpc(&h, "group.create", json!({ "name": "Team Poll Group" })).await;
    let gid_w = gw["result"]["conversation_id"].as_str().unwrap().to_string();
    let gid_n = gn["result"]["conversation_id"].as_str().unwrap().to_string();

    let poll_def = poll_definition("Lunch?", &["Pizza".into(), "Tacos".into()]);
    let poll_bytes = widget_body(&poll_def).unwrap().into_bytes();

    let m_poll_w = inbound_custom(
        "w-grp-1",
        &gid_w,
        "did:key:zCreator",
        1_000,
        WIDGET_CONTENT_TYPE,
        poll_bytes.clone(),
    );
    h.deliver(true, m_poll_w).await;

    let m_poll_n = inbound_custom(
        "w-grp-1",
        &gid_n,
        "did:key:zCreator",
        1_000,
        WIDGET_CONTENT_TYPE,
        poll_bytes,
    );
    h.deliver(false, m_poll_n).await;

    let ans_m1 = answer_body("w-grp-1", &json!({ "option": "o1" })).unwrap().into_bytes();
    let m_ans_w = inbound_custom(
        "ans-m1",
        &gid_w,
        "did:key:zMember1",
        2_000,
        WIDGET_ANSWER_CONTENT_TYPE,
        ans_m1.clone(),
    );
    h.deliver(true, m_ans_w).await;

    let m_ans_n = inbound_custom(
        "ans-m1",
        &gid_n,
        "did:key:zMember1",
        2_000,
        WIDGET_ANSWER_CONTENT_TYPE,
        ans_m1,
    );
    h.deliver(false, m_ans_n).await;

    let mut lw = one_rpc(
        &h,
        true,
        "widget.list",
        json!({ "conversation": gid_w, "widget_ids": ["w-grp-1"] }),
    )
    .await;
    let mut ln = one_rpc(
        &h,
        false,
        "widget.list",
        json!({ "conversation": gid_n, "widget_ids": ["w-grp-1"] }),
    )
    .await;
    strip_volatile(&mut lw);
    strip_volatile(&mut ln);
    normalize_message_ids(&mut lw);
    normalize_message_ids(&mut ln);
    assert_eq!(lw, ln);
    assert_eq!(
        lw["result"]["widgets"][0]["counts"],
        json!([
            { "field": "option", "choice": "o1", "count": 1 },
            { "field": "option", "choice": "o2", "count": 0 }
        ])
    );

    let svc = did_for_service("conversation");
    for (conv_svc, gid) in [(&h.wasm_conversation, &gid_w), (&h.native_conversation, &gid_n)] {
        let store = conv_svc.store_for(&svc).await.unwrap();
        let guard = store.conn().lock().unwrap();
        guard
            .execute(
                "UPDATE group_members SET removed_epoch = 99 WHERE conversation_id = ?1 AND \
                 member_address = ?2",
                [gid.as_str(), svc.as_str()],
            )
            .unwrap();
    }

    let w_ans_rem = one_rpc(
        &h,
        true,
        "widget.answer",
        json!({ "conversation": gid_w, "widget_id": "w-grp-1", "data": { "option": "o2" } }),
    )
    .await;
    let n_ans_rem = one_rpc(
        &h,
        false,
        "widget.answer",
        json!({ "conversation": gid_n, "widget_id": "w-grp-1", "data": { "option": "o2" } }),
    )
    .await;
    assert_eq!(w_ans_rem, n_ans_rem);
    assert_eq!(w_ans_rem["error"]["message"], group::GROUP_REMOVED_NOTICE);
}

#[tokio::test]
async fn scenario_264_inbound_bad_or_unknown_widgets_parity() {
    let h = harness().await;
    let conv = open_conv(&h, "did:key:zPeer264").await;

    let big_body = vec![b'a'; 8 * 1024 + 1];
    let m_big =
        inbound_custom("w-big", &conv, "did:key:zPeer", 1_000, WIDGET_CONTENT_TYPE, big_body);
    h.deliver(true, m_big.clone()).await;
    h.deliver(false, m_big).await;

    let bad_shape = json!({
        "widget_version": 1,
        "widget": { "semantic": "only_semantic" }
    })
    .to_string()
    .into_bytes();
    let m_bad_shape = inbound_custom(
        "w-bad-shape",
        &conv,
        "did:key:zPeer",
        2_000,
        WIDGET_CONTENT_TYPE,
        bad_shape,
    );
    h.deliver(true, m_bad_shape.clone()).await;
    h.deliver(false, m_bad_shape).await;

    let bad_field_type = json!({
        "widget_version": 1,
        "widget": {
            "extension": { "id": "poll", "version": 1 },
            "type": "poll/single",
            "semantic": "poll",
            "prompt": "Location?",
            "fields": [{ "name": "loc", "type": "location", "required": true }]
        }
    })
    .to_string()
    .into_bytes();
    let m_bft =
        inbound_custom("w-bft", &conv, "did:key:zPeer", 3_000, WIDGET_CONTENT_TYPE, bad_field_type);
    h.deliver(true, m_bft.clone()).await;
    h.deliver(false, m_bft).await;

    let unknown_type_def = json!({
        "widget_version": 1,
        "widget": {
            "extension": { "id": "feedback", "version": 1 },
            "type": "feedback/rating",
            "semantic": "rating",
            "prompt": "Score 1-5?",
            "fields": [{ "name": "score", "type": "number", "required": true, "min": 1, "max": 5 }]
        }
    })
    .to_string()
    .into_bytes();
    let m_unk = inbound_custom(
        "w-unk",
        &conv,
        "did:key:zPeer",
        4_000,
        WIDGET_CONTENT_TYPE,
        unknown_type_def,
    );
    h.deliver(true, m_unk.clone()).await;
    h.deliver(false, m_unk).await;

    let (mut lw, mut ln) = both_rpc(&h, "widget.list", json!({ "conversation": conv })).await;
    strip_volatile(&mut lw);
    strip_volatile(&mut ln);
    normalize_message_ids(&mut lw);
    normalize_message_ids(&mut ln);
    assert_eq!(lw, ln);

    let widgets = lw["result"]["widgets"].as_array().unwrap();
    assert_eq!(widgets.len(), 4);
    assert_eq!(widgets[0]["refused"], "widget-too-large");
    assert_eq!(widgets[1]["refused"], "widget-shape");
    assert_eq!(widgets[2]["refused"], "widget-shape");
    assert_eq!(widgets[3]["checked"], false);
    assert!(widgets[3]["refused"].is_null());

    let (ans_w, ans_n) = both_rpc(
        &h,
        "widget.answer",
        json!({ "conversation": conv, "widget_id": "w-unk", "data": { "score": 5 } }),
    )
    .await;
    assert!(ans_w["error"].is_null());
    assert!(ans_n["error"].is_null());

    let (mut lw2, mut ln2) =
        both_rpc(&h, "widget.list", json!({ "conversation": conv, "widget_ids": ["w-unk"] })).await;
    strip_volatile(&mut lw2);
    strip_volatile(&mut ln2);
    normalize_message_ids(&mut lw2);
    normalize_message_ids(&mut ln2);
    assert_eq!(lw2, ln2);

    let unk_view = &lw2["result"]["widgets"][0];
    assert_eq!(unk_view["answers"].as_array().unwrap().len(), 1);
    assert_eq!(unk_view["answers"][0]["data"], json!({ "score": 5 }));
}

#[tokio::test]
async fn scenario_265_widget_list_forward_scan_parity() {
    let h = harness().await;
    let conv = "conv-265";

    let poll = poll_definition("Q1", &["A".into(), "B".into()]);
    let poll_bytes = widget_body(&poll).unwrap().into_bytes();

    let m1 = inbound_custom(
        "w-scan-1",
        conv,
        "did:key:zPeer",
        1_000,
        WIDGET_CONTENT_TYPE,
        poll_bytes.clone(),
    );
    h.deliver(true, m1.clone()).await;
    h.deliver(false, m1).await;

    let m2 = inbound_custom(
        "w-scan-2",
        conv,
        "did:key:zPeer",
        2_000,
        WIDGET_CONTENT_TYPE,
        poll_bytes.clone(),
    );
    h.deliver(true, m2.clone()).await;
    h.deliver(false, m2).await;

    let (mut lw, mut ln) = both_rpc(
        &h,
        "widget.list",
        json!({ "conversation": conv, "widget_ids": ["w-scan-1", "missing-id"] }),
    )
    .await;
    strip_volatile(&mut lw);
    strip_volatile(&mut ln);
    normalize_message_ids(&mut lw);
    normalize_message_ids(&mut ln);
    assert_eq!(lw, ln);
    let listed = lw["result"]["widgets"].as_array().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["definition"]["prompt"], "Q1");

    let conv_cap = "conv-265-cap";
    let m_oldest = inbound_custom(
        "w-oldest",
        conv_cap,
        "did:key:zPeer",
        500,
        WIDGET_CONTENT_TYPE,
        poll_bytes.clone(),
    );
    h.deliver(true, m_oldest.clone()).await;
    h.deliver(false, m_oldest).await;

    for i in 1..=1000 {
        let msg = inbound(&format!("msg-{i}"), conv_cap, "did:key:zPeer", 1000 + i, "dummy text");
        h.deliver(true, msg.clone()).await;
        h.deliver(false, msg).await;
    }

    let (lw_cap, ln_cap) = both_rpc(
        &h,
        "widget.list",
        json!({ "conversation": conv_cap, "widget_ids": ["w-oldest"] }),
    )
    .await;
    assert_eq!(lw_cap, ln_cap);
    assert_eq!(lw_cap["result"]["complete"], false);

    let conv_unfiltered = "conv-265-unfiltered";
    for i in 1..=51 {
        let wid_msg = inbound_custom(
            &format!("w-unf-{i}"),
            conv_unfiltered,
            "did:key:zPeer",
            1000 + i,
            WIDGET_CONTENT_TYPE,
            poll_bytes.clone(),
        );
        h.deliver(true, wid_msg.clone()).await;
        h.deliver(false, wid_msg).await;
    }

    let (lw_unf, ln_unf) =
        both_rpc(&h, "widget.list", json!({ "conversation": conv_unfiltered })).await;
    assert_eq!(lw_unf, ln_unf);
    assert_eq!(lw_unf["result"]["widgets"].as_array().unwrap().len(), 50);
    assert_eq!(lw_unf["result"]["complete"], false);
}
