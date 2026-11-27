use serde_json::{Value, json};
use syneroym_roym_core::{
    backup::{BUNDLE_MANIFEST_VERSION, Bundle},
    chat::ROYM_MESSAGE_CONTENT_TYPE,
    conversation::group::GROUP_RESTORED_NOTICE,
    record::{Envelope, RECORD_BUNDLE_MANIFEST, RecordDraft},
};
use syneroym_rpc::ConversationHost;

use super::{fixtures::*, helpers::*};

fn conv_service() -> String {
    did_for_service("conversation")
}

async fn send_signed(h: &Harness, conv: &str, text: &str) -> (Value, Value) {
    both_rpc(
        h,
        "conversation.send",
        json!({
            "conversation": conv,
            "body": text,
            "languages": ["en"],
            "forward_mode": "signed",
        }),
    )
    .await
}

async fn history(h: &Harness, conv: &str) -> (Value, Value) {
    both_rpc(h, "conversation.history", json!({ "conversation": conv })).await
}

fn containers_match(w: &mut Value, n: &mut Value) {
    strip_volatile(w);
    strip_volatile(n);
    normalize_message_ids(w);
    normalize_message_ids(n);
}

#[tokio::test]
async fn scenario_278_settings_default_change_and_asked_once_parity() {
    let h = harness().await;

    let (w, n) = both_rpc(&h, "conversation.settings", json!({})).await;
    assert_eq!(w, n);
    assert_eq!(w["result"]["default_mode"], "not-forwardable");
    assert_eq!(w["result"]["asked"], false);

    let (w, n) = both_rpc(&h, "conversation.settings", json!({ "default_mode": "signed" })).await;
    assert_eq!(w, n);
    assert_eq!(w["result"]["default_mode"], "signed");
    assert_eq!(w["result"]["asked"], false);

    let (w, n) = both_rpc(&h, "conversation.settings", json!({ "asked": true })).await;
    assert_eq!(w, n);
    assert_eq!(w["result"]["default_mode"], "signed");
    assert_eq!(w["result"]["asked"], true);

    let (w, n) = both_rpc(&h, "conversation.settings", json!({})).await;
    assert_eq!(w, n);
    assert_eq!(w["result"]["default_mode"], "signed");
    assert_eq!(w["result"]["asked"], true);

    let (w, n) = both_rpc(&h, "conversation.settings", json!({ "asked": false })).await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "asked");

    let (w, n) = both_rpc(&h, "conversation.settings", json!({ "default_mode": "signd" })).await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32602);
}

#[tokio::test]
async fn scenario_279_signed_send_carries_a_checked_origin_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    let conv = open_conv(&h, "did:key:zPeer279").await;

    let (sw, sn) = send_signed(&h, &conv, "Market opens at 9").await;
    assert!(sw["result"]["message_id"].is_string(), "{sw}");
    assert!(sn["result"]["message_id"].is_string(), "{sn}");

    let (mut hw, mut hn) = history(&h, &conv).await;
    containers_match(&mut hw, &mut hn);
    assert_eq!(hw, hn);
    let row = &hw["result"]["messages"][0];
    assert_eq!(row["forward"]["mode"], "signed");
    assert_eq!(row["forward"]["forwardable"], true);
    assert_eq!(row["forward"]["origin"]["author_did"], h.owner_did);
    assert_eq!(row["forward"]["origin"]["is_you"], true);
    assert_eq!(row["forward"]["forwarded"], false);
    assert_eq!(row["body"], "Market opens at 9");

    let msg_id = sw["result"]["message_id"].as_str().unwrap();
    let raw = h.wasm_conversation.get_message(&conv_service(), msg_id).await.unwrap();
    assert_eq!(raw.content_type, ROYM_MESSAGE_CONTENT_TYPE);
    let stored: Value = serde_json::from_slice(&raw.body).unwrap();
    assert_eq!(stored["forward_mode"], "signed");
    assert_eq!(stored["text"], "Market opens at 9");
    assert!(stored["origin"].is_string());
}

#[tokio::test]
async fn scenario_280_not_forwardable_send_has_no_origin_parity() {
    let h = harness().await;
    let conv = open_conv(&h, "did:key:zPeer280").await;

    let (sw, sn) = both_rpc(
        &h,
        "conversation.send",
        json!({
            "conversation": conv,
            "body": "keep this quiet",
            "forward_mode": "not-forwardable",
        }),
    )
    .await;
    assert!(sw["result"]["message_id"].is_string(), "{sw}");
    assert!(sn["result"]["message_id"].is_string(), "{sn}");

    let (mut hw, mut hn) = history(&h, &conv).await;
    containers_match(&mut hw, &mut hn);
    assert_eq!(hw, hn);
    let row = &hw["result"]["messages"][0];
    assert_eq!(row["forward"]["mode"], "not-forwardable");
    assert_eq!(row["forward"]["forwardable"], false);
    assert!(row["forward"].get("origin").is_none(), "{row}");

    let msg_id = sw["result"]["message_id"].as_str().unwrap();
    let raw = h.wasm_conversation.get_message(&conv_service(), msg_id).await.unwrap();
    let stored: Value = serde_json::from_slice(&raw.body).unwrap();
    assert!(stored.get("origin").is_none(), "{stored}");
}

#[tokio::test]
async fn scenario_281_signed_send_without_a_certificate_is_refused_parity() {
    let h = harness().await;
    let conv = open_conv(&h, "did:key:zPeer281").await;

    let (w, n) = send_signed(&h, &conv, "no certificate").await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["code"], "signing-not-enrolled");

    let (ow, on) = both_rpc(&h, "conversation.outbox", json!({})).await;
    assert_eq!(ow, on);
    assert!(ow["result"]["outbox"].as_array().unwrap().is_empty(), "{ow}");
}

#[tokio::test]
async fn scenario_282_send_with_a_mode_refuses_another_type_and_a_bad_mode_parity() {
    let h = harness().await;
    let conv = open_conv(&h, "did:key:zPeer282").await;

    let (w, n) = both_rpc(
        &h,
        "conversation.send",
        json!({
            "conversation": conv,
            "body": "<b>hi</b>",
            "content_type": "text/html",
            "forward_mode": "signed",
        }),
    )
    .await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32602);
    assert_eq!(w["error"]["data"]["field"], "content_type");

    let (w, n) = both_rpc(
        &h,
        "conversation.send",
        json!({
            "conversation": conv,
            "body": "{}",
            "content_type": ROYM_MESSAGE_CONTENT_TYPE,
        }),
    )
    .await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32602);
    assert!(w["error"]["message"].as_str().unwrap().contains("reserved"), "{w}");

    let (w, n) = both_rpc(
        &h,
        "conversation.send",
        json!({ "conversation": conv, "body": "typo", "forward_mode": "signd" }),
    )
    .await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32602);
}

#[tokio::test]
async fn scenario_283_search_reads_the_chat_text_only_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    let conv = open_conv(&h, "did:key:zPeer283").await;

    send_signed(&h, &conv, "Market opens at nine").await;
    for body in ["Market news one", "Market news two", "Market news three"] {
        both_rpc(&h, "conversation.send", json!({ "conversation": conv, "body": body })).await;
    }

    for query in ["signed", "forward_mode", "envelope", "chat_version"] {
        let (w, n) = both_rpc(&h, "conversation.search", json!({ "query": query })).await;
        assert_eq!(w, n);
        assert!(w["result"]["matches"].as_array().unwrap().is_empty(), "{query}: {w}");
    }

    let (mut w, mut n) =
        both_rpc(&h, "conversation.search", json!({ "query": "Market", "limit": 10 })).await;
    containers_match(&mut w, &mut n);
    assert_eq!(w, n);
    assert_eq!(w["result"]["matches"].as_array().unwrap().len(), 4);
    assert!(
        w["result"]["matches"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["body"].as_str().is_some_and(|b| b.contains("Market")))
    );

    // The cap applies to real hits only: a chat body ahead of the cut must
    // not hide a later plain message. A newer chat body must also not hide
    // an older plain message that holds the same word: without the host
    // rule, the body's JSON would match too.
    let (mut w, mut n) =
        both_rpc(&h, "conversation.search", json!({ "query": "Market", "limit": 2 })).await;
    containers_match(&mut w, &mut n);
    assert_eq!(w, n);
    assert_eq!(w["result"]["matches"].as_array().unwrap().len(), 2);

    both_rpc(&h, "conversation.send", json!({ "conversation": conv, "body": "I signed the form" }))
        .await;
    send_signed(&h, &conv, "Please note this").await;
    let (mut w, mut n) =
        both_rpc(&h, "conversation.search", json!({ "query": "signed", "limit": 1 })).await;
    containers_match(&mut w, &mut n);
    assert_eq!(w, n);
    let matches = w["result"]["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["body"], "I signed the form");
    assert_eq!(matches[0]["content_type"], "text/plain");
}

#[tokio::test]
async fn scenario_284_settings_travel_in_the_bundle_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    both_rpc(&h, "conversation.settings", json!({ "default_mode": "signed", "asked": true })).await;

    let (exp_w, exp_n) = both_rpc(&h, "conversation.export", json!({})).await;
    assert_eq!(exp_w, exp_n);
    let settings = exp_w["result"]["sections"]["settings"].as_array().unwrap();
    assert_eq!(settings.len(), 1);
    assert_eq!(settings[0]["id"], "forward");
    assert_eq!(settings[0]["payload"]["default_mode"], "signed");

    // A fresh node gets the choice back from the bundle and does not ask
    // again.
    let restored = harness().await;
    let (iw, in_) =
        both_rpc(&restored, "conversation.import", json!({ "bundle": exp_w["result"] })).await;
    assert_eq!(iw, in_);
    assert!(!is_err(&iw, -32602), "import failed: {iw}");

    let (w, n) = both_rpc(&restored, "conversation.settings", json!({})).await;
    assert_eq!(w, n);
    assert_eq!(w["result"]["default_mode"], "signed");
    assert_eq!(w["result"]["asked"], true);

    // A settings row of another id is refused, even with a valid manifest.
    let mut bundle: Bundle = serde_json::from_value(exp_w["result"].clone()).unwrap();
    bundle.sections.get_mut("settings").unwrap()[0]["id"] = json!("other-setting");
    let digest = Bundle::digest(
        syneroym_roym_conversation::app::SCHEMA_VERSION,
        bundle.sections.get("settings").unwrap(),
    )
    .unwrap();
    bundle.manifest.sections.insert("settings".to_string(), digest);
    resign_bundle(&mut bundle, &h.owner, wall_now());
    let (w, n) = both_rpc(&h, "conversation.import", json!({ "bundle": bundle })).await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32602);
    assert!(w["error"]["message"].as_str().unwrap().contains("other-setting"), "{w}");
}

#[tokio::test]
async fn scenario_285_signed_send_to_a_restored_group_signs_nothing_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    let (gw, gn) = both_rpc(&h, "group.create", json!({ "name": "Left Behind" })).await;
    let group_w = gw["result"]["conversation_id"].as_str().unwrap().to_string();
    let group_n = gn["result"]["conversation_id"].as_str().unwrap().to_string();
    let (exp_w, exp_n) = both_rpc(&h, "conversation.export", json!({})).await;

    let restored = harness().await;
    let iw =
        one_rpc(&restored, true, "conversation.import", json!({ "bundle": exp_w["result"] })).await;
    let in_ =
        one_rpc(&restored, false, "conversation.import", json!({ "bundle": exp_n["result"] }))
            .await;
    assert!(iw["result"].is_object(), "{iw}");
    assert!(in_["result"].is_object(), "{in_}");

    let signed_w = one_rpc(
        &restored,
        true,
        "conversation.send",
        json!({ "conversation": group_w, "body": "still here", "forward_mode": "signed" }),
    )
    .await;
    let signed_n = one_rpc(
        &restored,
        false,
        "conversation.send",
        json!({ "conversation": group_n, "body": "still here", "forward_mode": "signed" }),
    )
    .await;
    // No certificate is enrolled on the restored node, so a signing call
    // before the group check would answer `signing-not-enrolled` instead.
    assert_eq!(signed_w["error"]["message"], GROUP_RESTORED_NOTICE);
    assert_eq!(signed_n["error"]["message"], GROUP_RESTORED_NOTICE);

    for stack in [&restored.wasm_conversation, &restored.native_conversation] {
        let outbox = stack.outbox(&conv_service()).await.unwrap();
        assert!(outbox.is_empty(), "a refused send must leave nothing queued");
    }
}

async fn raw_body(h: &Harness, wasm: bool, message_id: &str) -> Value {
    let stack = if wasm { &h.wasm_conversation } else { &h.native_conversation };
    let msg = stack.get_message(&conv_service(), message_id).await.unwrap();
    serde_json::from_slice(&msg.body).unwrap()
}

#[tokio::test]
async fn scenario_286_forward_copies_the_origin_and_adds_one_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    let source = open_conv(&h, "did:key:zPeer286a").await;
    let target = open_conv(&h, "did:key:zPeer286b").await;
    // The forwarder's own default must not apply to the author's content.
    both_rpc(&h, "conversation.settings", json!({ "default_mode": "not-forwardable" })).await;

    let (sw, sn) = send_signed(&h, &source, "Forward me, please").await;
    let src_w = sw["result"]["message_id"].as_str().unwrap();
    let src_n = sn["result"]["message_id"].as_str().unwrap();
    let origin_w = raw_body(&h, true, src_w).await["origin"].clone();
    let origin_n = raw_body(&h, false, src_n).await["origin"].clone();

    let fw = one_rpc(
        &h,
        true,
        "conversation.forward",
        json!({ "message_id": src_w, "conversation": target }),
    )
    .await;
    let fnat = one_rpc(
        &h,
        false,
        "conversation.forward",
        json!({ "message_id": src_n, "conversation": target }),
    )
    .await;
    assert!(fw["result"]["message_id"].is_string(), "{fw}");
    assert!(fnat["result"]["message_id"].is_string(), "{fnat}");

    let new_w = fw["result"]["message_id"].as_str().unwrap();
    let new_n = fnat["result"]["message_id"].as_str().unwrap();
    let body_w = raw_body(&h, true, new_w).await;
    let body_n = raw_body(&h, false, new_n).await;
    assert_eq!(body_w["origin"], origin_w);
    assert_eq!(body_n["origin"], origin_n);
    assert_eq!(body_w["forward_count"], 1);
    assert_eq!(body_n["forward_count"], 1);
    assert_eq!(body_w["forward_mode"], "signed");
    assert_eq!(body_w["text"], "Forward me, please");

    let (mut hw, mut hn) = history(&h, &target).await;
    containers_match(&mut hw, &mut hn);
    assert_eq!(hw, hn);
    let row = &hw["result"]["messages"][0];
    assert_eq!(row["forward"]["mode"], "signed");
    assert_eq!(row["forward"]["forwardable"], true);
    assert_eq!(row["forward"]["forwarded"], true);
    assert_eq!(row["forward"]["forward_count"], 1);
    assert_eq!(row["forward"]["origin"]["author_did"], h.owner_did);
}

#[tokio::test]
async fn scenario_287_forward_refuses_every_other_message_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    let conv = open_conv(&h, "did:key:zPeer287").await;
    let target = open_conv(&h, "did:key:zPeer287b").await;

    async fn refusal(h: &Harness, wasm: bool, id: &str, target: &str) -> Value {
        one_rpc(
            h,
            wasm,
            "conversation.forward",
            json!({ "message_id": id, "conversation": target }),
        )
        .await
    }

    let (pw, pn) =
        both_rpc(&h, "conversation.send", json!({ "conversation": conv, "body": "plain words" }))
            .await;
    let plain_w = pw["result"]["message_id"].as_str().unwrap();
    let plain_n = pn["result"]["message_id"].as_str().unwrap();
    for (wasm, id) in [(true, plain_w), (false, plain_n)] {
        let r = refusal(&h, wasm, id, &target).await;
        assert_eq!(r["error"]["data"]["code"], "not-forwardable", "{r}");
    }

    let (nw, nn) = both_rpc(
        &h,
        "conversation.send",
        json!({ "conversation": conv, "body": "not for sharing", "forward_mode": "not-forwardable" }),
    )
    .await;
    let nf_w = nw["result"]["message_id"].as_str().unwrap();
    let nf_n = nn["result"]["message_id"].as_str().unwrap();
    for (wasm, id) in [(true, nf_w), (false, nf_n)] {
        let r = refusal(&h, wasm, id, &target).await;
        assert_eq!(r["error"]["data"]["code"], "not-forwardable", "{r}");
    }

    let poll = syneroym_roym_core::widget::poll_definition(
        "Question?",
        &["A".to_string(), "B".to_string()],
    );
    let (ww, wn) =
        both_rpc(&h, "widget.send", json!({ "conversation": conv, "widget": poll })).await;
    let widget_w = ww["result"]["message_id"].as_str().unwrap();
    let widget_n = wn["result"]["message_id"].as_str().unwrap();
    for (wasm, id) in [(true, widget_w), (false, widget_n)] {
        let r = refusal(&h, wasm, id, &target).await;
        assert_eq!(r["error"]["data"]["code"], "not-forwardable", "{r}");
    }

    let (sw, sn) = send_signed(&h, &conv, "delete me").await;
    let del_w = sw["result"]["message_id"].as_str().unwrap();
    let del_n = sn["result"]["message_id"].as_str().unwrap();
    one_rpc(&h, true, "conversation.delete-message", json!({ "message_id": del_w })).await;
    one_rpc(&h, false, "conversation.delete-message", json!({ "message_id": del_n })).await;
    for (wasm, id) in [(true, del_w), (false, del_n)] {
        let r = refusal(&h, wasm, id, &target).await;
        assert_eq!(r["error"]["data"]["code"], "message-deleted", "{r}");
    }

    // A chat body stored through the host with a broken origin is refused at
    // the origin check, not at the content-type step.
    let (bw, bn) = send_signed(&h, &conv, "Market opens at 9").await;
    for (wasm, id) in [
        (true, bw["result"]["message_id"].as_str().unwrap()),
        (false, bn["result"]["message_id"].as_str().unwrap()),
    ] {
        let mut body = raw_body(&h, wasm, id).await;
        body["text"] = json!("Market opens at 10");
        let stack = if wasm { &h.wasm_conversation } else { &h.native_conversation };
        let stored = stack
            .send(
                &conv_service(),
                &conv,
                ROYM_MESSAGE_CONTENT_TYPE,
                vec!["en".to_string()],
                body.to_string().into_bytes(),
            )
            .await
            .unwrap();
        let r = refusal(&h, wasm, &stored, &target).await;
        assert_eq!(r["error"]["data"]["code"], "origin-not-checked", "{r}");
    }

    for wasm in [true, false] {
        let r = refusal(&h, wasm, "msg:no-such-message", &target).await;
        assert_eq!(r["error"]["data"]["code"], "message-not-found", "{r}");
    }
}

#[tokio::test]
async fn scenario_288_forwarding_a_forward_keeps_the_count_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    let source = open_conv(&h, "did:key:zPeer288a").await;
    let target = open_conv(&h, "did:key:zPeer288b").await;

    let (sw, sn) = send_signed(&h, &source, "One hop").await;
    let src_w = sw["result"]["message_id"].as_str().unwrap();
    let src_n = sn["result"]["message_id"].as_str().unwrap();

    let f1w = one_rpc(
        &h,
        true,
        "conversation.forward",
        json!({ "message_id": src_w, "conversation": target }),
    )
    .await;
    let f1n = one_rpc(
        &h,
        false,
        "conversation.forward",
        json!({ "message_id": src_n, "conversation": target }),
    )
    .await;
    let hop_w = f1w["result"]["message_id"].as_str().unwrap();
    let hop_n = f1n["result"]["message_id"].as_str().unwrap();

    // Forward the node's own forward into the same conversation.
    let f2w = one_rpc(
        &h,
        true,
        "conversation.forward",
        json!({ "message_id": hop_w, "conversation": target }),
    )
    .await;
    let f2n = one_rpc(
        &h,
        false,
        "conversation.forward",
        json!({ "message_id": hop_n, "conversation": target }),
    )
    .await;
    assert!(f2w["result"]["message_id"].is_string(), "{f2w}");
    assert!(f2n["result"]["message_id"].is_string(), "{f2n}");
    assert_eq!(
        raw_body(&h, true, f2w["result"]["message_id"].as_str().unwrap()).await["forward_count"],
        1
    );

    let (mut hw, mut hn) = history(&h, &target).await;
    containers_match(&mut hw, &mut hn);
    assert_eq!(hw, hn);
    let counts: Vec<i64> = hw["result"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["forward"]["forward_count"].as_i64().unwrap())
        .collect();
    assert_eq!(counts, vec![1, 1]);
}

fn resign_bundle(bundle: &mut Bundle, owner: &syneroym_identity::Identity, now_secs: u64) {
    let payload = serde_json::to_value(&bundle.manifest).unwrap();
    let draft = RecordDraft {
        version: BUNDLE_MANIFEST_VERSION,
        record_type: RECORD_BUNDLE_MANIFEST.to_string(),
        subject: bundle.manifest.subject_did.clone(),
        payload,
        expires_at_secs: None,
        supersedes: None,
    };
    let owner_did = syneroym_identity::substrate::derive_did_key(&owner.public_key());
    let (mut env, bytes) = Envelope::unsigned(draft, owner_did, None, now_secs).unwrap();
    let sig = z32::encode(&owner.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    bundle.manifest_signature = Some(env.to_json().unwrap());
}
