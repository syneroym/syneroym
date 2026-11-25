use serde_json::{Value, json};
use syneroym_roym_core::{
    conversation::group,
    widget::{
        WIDGET_ANSWER_CONTENT_TYPE, WIDGET_CONTENT_TYPE, WidgetBody, answer_body, poll_definition,
        widget_body,
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

async fn deliver_both(h: &Harness, m: ConversationMessage) {
    h.deliver(true, m.clone()).await;
    h.deliver(false, m).await;
}

async fn deliver_poll(h: &Harness, id: &str, conv: &str, ts: i64, body: &[u8]) {
    deliver_both(
        h,
        inbound_custom(id, conv, "did:key:zPeer", ts, WIDGET_CONTENT_TYPE, body.to_vec()),
    )
    .await;
}

async fn deliver_ans(
    h: &Harness,
    id: &str,
    conv: &str,
    wid: &str,
    opt: &str,
    author: &str,
    ts: i64,
) {
    let b = answer_body(wid, &json!({ "option": opt })).unwrap().into_bytes();
    deliver_both(h, inbound_custom(id, conv, author, ts, WIDGET_ANSWER_CONTENT_TYPE, b)).await;
}

fn normalize_views(w: &mut Value, n: &mut Value) {
    strip_volatile(w);
    strip_volatile(n);
    normalize_message_ids(w);
    normalize_message_ids(n);
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
    normalize_views(&mut lw, &mut ln);
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
    normalize_views(&mut lw_all, &mut ln_all);
    assert_eq!(lw_all, ln_all);
    assert_eq!(lw_all["result"]["widgets"].as_array().unwrap().len(), 1);
    assert_eq!(lw_all["result"]["complete"], true);
}

async fn deliver_scenario_261_inbound(h: &Harness, conv: &str) {
    let poll = poll_definition("Which path?", &["Left".into(), "Right".into()]);
    let poll_bytes = widget_body(&poll).unwrap().into_bytes();
    deliver_poll(h, "w1", conv, 1_000, &poll_bytes).await;
    deliver_ans(h, "ans-a1", conv, "w1", "o1", "did:key:zVoterA", 2_000).await;
    deliver_ans(h, "ans-a2", conv, "w1", "o2", "did:key:zVoterA", 3_000).await;
    deliver_ans(h, "ans-b1", conv, "w1", "o1", "did:key:zVoterB", 2_500).await;
    deliver_ans(h, "ans-b-bad", conv, "w1", "o99", "did:key:zVoterB", 3_500).await;
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

    let esc_prompt = "\u{1}".repeat(500);
    let esc_opts: Vec<String> = (1..=12).map(|_| "\u{1}".repeat(120)).collect();
    let large_poll = poll_definition(&esc_prompt, &esc_opts);
    let (w_lg, n_lg) =
        both_rpc(h, "widget.send", json!({ "conversation": conv, "widget": large_poll })).await;
    assert_eq!(w_lg, n_lg);
    assert_eq!(w_lg["error"]["data"]["code"], "widget-too-large");

    let big_prompt = "x".repeat(8 * 1024);
    let mut large_def = poll_definition("Q?", &["A".into(), "B".into()]);
    large_def.prompt = big_prompt;
    let (w_shp, n_shp) =
        both_rpc(h, "widget.send", json!({ "conversation": conv, "widget": large_def })).await;
    assert_eq!(w_shp, n_shp);
    assert_eq!(w_shp["error"]["data"]["code"], "widget-shape");

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

async fn assert_inbound_widget_refusals(h: &Harness, conv: &str) {
    let unk_widget = json!({
        "widget_version": 1,
        "widget": {
            "extension": { "id": "survey", "version": 1 },
            "type": "survey/long",
            "semantic": "form",
            "prompt": "Fill",
            "fields": (0..5).map(|i| json!({ "name": format!("f{i}"), "type": "long-text", "required": false, "max_len": 2048 })).collect::<Vec<_>>()
        }
    }).to_string().into_bytes();
    let m_unk_form = inbound_custom(
        "w-form-lg",
        conv,
        "did:key:zPeer262",
        1_100,
        WIDGET_CONTENT_TYPE,
        unk_widget,
    );
    deliver_both(h, m_unk_form).await;

    let mut lg_ans_data = serde_json::Map::new();
    for i in 0..5 {
        lg_ans_data.insert(format!("f{i}"), json!("a".repeat(1800)));
    }
    let (w_ans_lg, n_ans_lg) = both_rpc(
        h,
        "widget.answer",
        json!({ "conversation": conv, "widget_id": "w-form-lg", "data": lg_ans_data }),
    )
    .await;
    assert_eq!(w_ans_lg, n_ans_lg);
    assert_eq!(w_ans_lg["error"]["data"]["code"], "answer-too-large");

    let poll_def = poll_definition("Choice?", &["A".into(), "B".into()]);
    let poll_bytes = widget_body(&poll_def).unwrap().into_bytes();
    let m_unadmitted = inbound_custom(
        "w-unadmitted",
        conv,
        "did:key:zPeer262",
        1_200,
        WIDGET_CONTENT_TYPE,
        poll_bytes,
    );
    deliver_both(h, m_unadmitted).await;

    let svc = did_for_service("conversation");
    for conv_svc in [&h.wasm_conversation, &h.native_conversation] {
        let store = conv_svc.store_for(&svc).await.unwrap();
        let guard = store.conn().lock().unwrap();
        guard
            .execute(
                "UPDATE messages SET visible_seq = 0, admission = 'undecided' WHERE id = \
                 'w-unadmitted'",
                [],
            )
            .unwrap();
    }

    let (w_unad_ans, n_unad_ans) = both_rpc(
        h,
        "widget.answer",
        json!({ "conversation": conv, "widget_id": "w-unadmitted", "data": { "option": "o1" } }),
    )
    .await;
    assert_eq!(w_unad_ans, n_unad_ans);
    assert_eq!(w_unad_ans["error"]["data"]["code"], "widget-not-found");

    let opts20: Vec<String> = (1..=20).map(|i| format!("Opt {i}")).collect();
    let p20 = poll_definition("Many?", &opts20);
    let p20_bytes = serde_json::to_vec(&WidgetBody { widget_version: 1, widget: p20 }).unwrap();
    let m_20 = inbound_custom(
        "w-20-opts",
        conv,
        "did:key:zPeer262",
        1_300,
        WIDGET_CONTENT_TYPE,
        p20_bytes,
    );
    deliver_both(h, m_20).await;

    let (w_ans_20, n_ans_20) = both_rpc(
        h,
        "widget.answer",
        json!({ "conversation": conv, "widget_id": "w-20-opts", "data": { "option": "o1" } }),
    )
    .await;
    assert_eq!(w_ans_20, n_ans_20);
    assert_eq!(w_ans_20["error"]["data"]["code"], "widget-refused");
}

#[tokio::test]
async fn scenario_262_widget_refusals_parity() {
    let h = harness().await;
    let conv = open_conv(&h, "did:key:zPeer262").await;
    let conv_other = open_conv(&h, "did:key:zPeer262Other").await;

    assert_widget_send_refusals(&h, &conv).await;

    let m_txt = inbound("txt-msg-262", &conv, "did:key:zPeer262", 1_000, "hello");
    deliver_both(&h, m_txt).await;

    let (w_not_w, n_not_w) = both_rpc(
        &h,
        "widget.answer",
        json!({ "conversation": conv, "widget_id": "txt-msg-262", "data": { "option": "o1" } }),
    )
    .await;
    assert_eq!(w_not_w, n_not_w);
    assert_eq!(w_not_w["error"]["data"]["code"], "not-a-widget");

    assert_inbound_widget_refusals(&h, &conv).await;

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

    let m_poll = |gid: &str| {
        inbound_custom(
            "w-grp-1",
            gid,
            "did:key:zCreator",
            1_000,
            WIDGET_CONTENT_TYPE,
            poll_bytes.clone(),
        )
    };
    h.deliver(true, m_poll(&gid_w)).await;
    h.deliver(false, m_poll(&gid_n)).await;

    let ans_m1 = answer_body("w-grp-1", &json!({ "option": "o1" })).unwrap().into_bytes();
    let m_ans = |gid: &str| {
        inbound_custom(
            "ans-m1",
            gid,
            "did:key:zMember1",
            2_000,
            WIDGET_ANSWER_CONTENT_TYPE,
            ans_m1.clone(),
        )
    };
    h.deliver(true, m_ans(&gid_w)).await;
    h.deliver(false, m_ans(&gid_n)).await;

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
    normalize_views(&mut lw, &mut ln);
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
    normalize_views(&mut lw, &mut ln);
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
    normalize_views(&mut lw2, &mut ln2);
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

    deliver_ans(&h, "ans-early", conv, "w-scan-1", "o1", "did:key:zEarly", 500).await;
    deliver_poll(&h, "w-scan-1", conv, 1_000, &poll_bytes).await;
    deliver_ans(&h, "ans-w2", conv, "w-scan-2", "o2", "did:key:zMid", 1_500).await;
    deliver_poll(&h, "w-scan-2", conv, 2_000, &poll_bytes).await;

    for i in 1..=205 {
        deliver_both(&h, inbound(&format!("pad-{i}"), conv, "did:key:zPeer", 2_000 + i, "pad"))
            .await;
    }
    deliver_ans(&h, "ans-late", conv, "w-scan-1", "o1", "did:key:zLate", 2_300).await;

    deliver_poll(&h, "w-del", conv, 2_400, &poll_bytes).await;
    both_rpc(
        &h,
        "conversation.delete-message",
        json!({ "message_id": "w-del", "ask_peer": false }),
    )
    .await;

    let (mut lw, mut ln) = both_rpc(
        &h,
        "widget.list",
        json!({ "conversation": conv, "widget_ids": ["w-scan-1", "w-scan-1", "w-scan-2", "w-del", "missing-id"] }),
    )
    .await;
    normalize_views(&mut lw, &mut ln);
    assert_eq!(lw, ln);
    let listed = lw["result"]["widgets"].as_array().unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0]["definition"]["prompt"], "Q1");
    assert_eq!(listed[0]["counts"][0]["count"], 1);
    assert_eq!(listed[1]["definition"]["prompt"], "Q1");
    assert_eq!(listed[1]["counts"][1]["count"], 1);

    assert_scan_cap_and_unfiltered(&h, &poll_bytes).await;
}

async fn assert_scan_cap_and_unfiltered(h: &Harness, poll_bytes: &[u8]) {
    let conv_cap = "conv-265-cap";
    deliver_poll(h, "w-oldest", conv_cap, 500, poll_bytes).await;
    for i in 1..=1000 {
        deliver_both(h, inbound(&format!("msg-{i}"), conv_cap, "did:key:zPeer", 1000 + i, "text"))
            .await;
    }
    let (lw_1000, ln_1000) =
        both_rpc(h, "widget.list", json!({ "conversation": conv_cap, "widget_ids": ["w-oldest"] }))
            .await;
    assert_eq!(lw_1000, ln_1000);
    assert_eq!(lw_1000["result"]["complete"], true);

    let svc = did_for_service("conversation");
    for conv_svc in [&h.wasm_conversation, &h.native_conversation] {
        let store = conv_svc.store_for(&svc).await.unwrap();
        let guard = store.conn().lock().unwrap();
        guard
            .execute(
                "INSERT INTO dag_entries (entry_id, conversation_id, author, sender_timestamp, \
                 epoch, kind, header, ciphertext, nonce, payload, signature, applied) VALUES \
                 ('grp-evt-1', ?1, 'did:key:zCreator', 2500, 1, 'membership', X'', NULL, NULL, \
                 NULL, zeroblob(64), 1)",
                [conv_cap],
            )
            .unwrap();
    }
    let (lw_grp, ln_grp) =
        both_rpc(h, "widget.list", json!({ "conversation": conv_cap, "widget_ids": ["w-oldest"] }))
            .await;
    assert_eq!(lw_grp, ln_grp);
    assert_eq!(lw_grp["result"]["complete"], true);

    deliver_both(h, inbound("msg-1001", conv_cap, "did:key:zPeer", 2600, "extra")).await;
    let (lw_1001, ln_1001) =
        both_rpc(h, "widget.list", json!({ "conversation": conv_cap, "widget_ids": ["w-oldest"] }))
            .await;
    assert_eq!(lw_1001, ln_1001);
    assert_eq!(lw_1001["result"]["complete"], false);

    let conv_unf = "conv-265-unfiltered";
    for i in 1..=51 {
        deliver_poll(h, &format!("w-unf-{i}"), conv_unf, 1000 + i, poll_bytes).await;
    }
    for i in 1..=205 {
        deliver_both(h, inbound(&format!("pad-u-{i}"), conv_unf, "did:key:zPeer", 1100 + i, "pad"))
            .await;
    }
    deliver_ans(h, "ans-unf-1", conv_unf, "w-unf-1", "o1", "did:key:zAns", 1400).await;

    let (lw_unf, ln_unf) = both_rpc(h, "widget.list", json!({ "conversation": conv_unf })).await;
    assert_eq!(lw_unf, ln_unf);
    assert_eq!(lw_unf["result"]["widgets"].as_array().unwrap().len(), 50);
    assert_eq!(lw_unf["result"]["complete"], false);
    assert_eq!(lw_unf["result"]["widgets"][0]["widget_id"], "w-unf-1");
    assert_eq!(lw_unf["result"]["widgets"][0]["counts"][0]["count"], 1);
}

#[tokio::test]
async fn scenario_266_widget_list_timing_parity() {
    let h = harness().await;
    let conv = "conv-266-timing";
    let poll = poll_definition("Q?", &["A".into(), "B".into()]);
    let poll_bytes = widget_body(&poll).unwrap().into_bytes();

    for i in 1..=200 {
        deliver_poll(&h, &format!("w-time-{i}"), conv, i as i64, &poll_bytes).await;
    }
    for i in 201..=1000 {
        deliver_both(
            &h,
            inbound(&format!("msg-time-{i}"), conv, "did:key:zPeer", i as i64, "chat"),
        )
        .await;
    }
    let ids: Vec<String> = (1..=200).map(|i| format!("w-time-{i}")).collect();

    let t_native = std::time::Instant::now();
    let ln =
        one_rpc(&h, false, "widget.list", json!({ "conversation": conv, "widget_ids": ids })).await;
    let d_native_ms = t_native.elapsed().as_millis();

    let t_wasm = std::time::Instant::now();
    let lw =
        one_rpc(&h, true, "widget.list", json!({ "conversation": conv, "widget_ids": ids })).await;
    let d_wasm_ms = t_wasm.elapsed().as_millis();

    println!(
        "STOP_RULE_MEASUREMENT: widget.list 1000 messages {}ms native / {}ms wasm (< 200ms limit)",
        d_native_ms, d_wasm_ms
    );
    assert_eq!(lw["result"]["widgets"].as_array().unwrap().len(), 200);
    assert_eq!(ln["result"]["widgets"].as_array().unwrap().len(), 200);
    assert!(d_wasm_ms < 2000, "WASM widget.list took {d_wasm_ms}ms (>= 2000ms limit)");
}
