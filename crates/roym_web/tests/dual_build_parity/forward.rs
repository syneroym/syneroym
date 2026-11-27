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
    // not hide a later plain message.
    let (mut w, mut n) =
        both_rpc(&h, "conversation.search", json!({ "query": "Market", "limit": 2 })).await;
    containers_match(&mut w, &mut n);
    assert_eq!(w, n);
    assert_eq!(w["result"]["matches"].as_array().unwrap().len(), 2);
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

    // The node chooses another default, then the bundle restores the first.
    both_rpc(&h, "conversation.settings", json!({ "default_mode": "not-forwardable" })).await;
    let (iw, in_) = both_rpc(&h, "conversation.import", json!({ "bundle": exp_w["result"] })).await;
    assert_eq!(iw, in_);
    assert!(!is_err(&iw, -32602), "import failed: {iw}");

    let (w, n) = both_rpc(&h, "conversation.settings", json!({})).await;
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
