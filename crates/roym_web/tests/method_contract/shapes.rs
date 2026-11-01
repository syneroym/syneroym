use std::{env, fs, path::PathBuf};

use serde_json::{Map, Value, json};
use syneroym_identity::{
    delegation::{DelegationCertificate, SCOPE_RECORD_SIGNING},
    substrate::resolve_did_key,
};
use syneroym_rpc::ConversationDeliveryState;

use super::{
    fixtures::inbound,
    helpers::{Harness, HttpDriver, did_for_service, harness},
};

fn is_closed_word_key(k: &str) -> bool {
    matches!(
        k,
        "kind"
            | "state"
            | "direction"
            | "body_encoding"
            | "admission"
            | "status"
            | "content_type"
            | "subject_kind"
            | "category"
            | "note"
            | "role"
            | "pair"
            | "next"
            | "payment_timing"
            | "track"
            | "writer"
            | "conflict"
            | "card_type"
            | "revocation_status"
            | "payment"
            | "fulfilment"
            | "where"
            | "event"
            | "auth"
    )
}

fn shape_inner(v: &Value, key: Option<&str>, in_notices: bool) -> Value {
    match v {
        Value::Null => Value::String("null".to_string()),
        Value::Bool(_) => Value::String("boolean".to_string()),
        Value::Number(_) => Value::String("number".to_string()),
        Value::String(s) => {
            if in_notices || key.is_some_and(is_closed_word_key) {
                Value::String(s.clone())
            } else {
                Value::String("string".to_string())
            }
        }
        Value::Array(arr) => {
            let mut items: Vec<Value> =
                arr.iter().map(|item| shape_inner(item, None, in_notices)).collect();
            items.sort_by(|a, b| {
                serde_json::to_string(a).unwrap().cmp(&serde_json::to_string(b).unwrap())
            });
            items.dedup();
            Value::Array(items)
        }
        Value::Object(map) => {
            let mut sorted_keys: Vec<&String> = map.keys().collect();
            sorted_keys.sort();
            let mut out = Map::new();
            for k in sorted_keys {
                let val = &map[k];
                let notices = in_notices || k == "notices";
                out.insert(k.clone(), shape_inner(val, Some(k.as_str()), notices));
            }
            Value::Object(out)
        }
    }
}

pub fn shape(v: &Value) -> Value {
    shape_inner(v, None, false)
}

pub(super) fn assert_shape(case: &str, result: &Value) {
    let s = shape(result);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/method_shapes");
    let path = dir.join(format!("{case}.json"));
    if env::var("UPDATE_METHOD_SHAPES").as_deref() == Ok("1") {
        fs::create_dir_all(&dir).unwrap();
        let formatted = serde_json::to_string_pretty(&s).unwrap() + "\n";
        fs::write(&path, formatted).unwrap();
    } else {
        let existing = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read shape file {}: {e}", path.display()));
        let expected: Value = serde_json::from_str(&existing)
            .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()));
        assert_eq!(s, expected, "shape mismatch for case '{case}'");
    }
}

pub(super) async fn rpc(h: &Harness, method: &str, params: Value) -> Value {
    let req = json!({ "method": method, "params": params }).to_string().into_bytes();
    let r = h.native_http.post("/rpc", req, Some(h.caller())).await;
    let resp: Value = serde_json::from_slice(&r.body).unwrap();
    if let Some(err) = resp.get("error") {
        panic!("rpc {method} returned error: {err}");
    }
    resp.get("result").cloned().unwrap_or(Value::Null)
}

pub(super) async fn enrol_native_signing(h: &Harness, service: &str) -> (Value, Value, Value) {
    let status_m = format!("{service}.signing-status");
    let install_m = format!("{service}.install-signing-certificate");
    let unconf = rpc(h, &status_m, json!({})).await;
    let signing_did = unconf["signing_did"].as_str().unwrap();
    let signing_pubkey = resolve_did_key(signing_did).unwrap();
    let cert = DelegationCertificate::issue(
        &h.owner,
        signing_pubkey,
        86_400 * 365 * 4,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();
    let installed = rpc(h, &install_m, json!({ "certificate": cert.to_json().unwrap() })).await;
    let conf = rpc(h, &status_m, json!({})).await;
    (unconf, installed, conf)
}

#[tokio::test]
async fn profile_shapes() {
    let h = harness().await;

    let ping = rpc(&h, "profile.ping", json!({})).await;
    assert_shape("profile_ping", &ping);

    let policy = rpc(&h, "profile.policy", json!({})).await;
    assert_shape("profile_policy", &policy);

    let (st_unconf, inst, st_conf) = enrol_native_signing(&h, "profile").await;
    assert_shape("profile_signing_status_unconfigured", &st_unconf);
    assert_shape("profile_install_signing_certificate", &inst);
    assert_shape("profile_signing_status", &st_conf);

    let get_empty = rpc(&h, "profile.get", json!({})).await;
    assert_shape("profile_get_empty", &get_empty);

    let set_res = rpc(
        &h,
        "profile.set",
        json!({
            "display_name": "Parity Alice",
            "about": "Testing profile",
            "conversation_address": "syneroym://addr/alice",
            "locale": "en-GB"
        }),
    )
    .await;
    assert_shape("profile_set", &set_res);

    let get_set = rpc(&h, "profile.get", json!({})).await;
    assert_shape("profile_get_set", &get_set);

    let export_res = rpc(&h, "profile.export", json!({})).await;
    assert_shape("profile_export", &export_res);

    let import_res = rpc(&h, "profile.import", json!({ "bundle": export_res })).await;
    assert_shape("profile_import", &import_res);
}

async fn contacts_shapes(h: &Harness) {
    let lim = rpc(h, "contacts.limits", json!({})).await;
    assert_shape("contacts_limits", &lim);

    let set_lim = rpc(
        h,
        "contacts.set-limits",
        json!({
            "window_secs": 3600,
            "max_per_window": 10
        }),
    )
    .await;
    assert_shape("contacts_set_limits", &set_lim);

    let c_list_empty = rpc(h, "contacts.list", json!({})).await;
    assert_shape("contacts_list_empty", &c_list_empty);

    let c_get_missing =
        rpc(h, "contacts.get", json!({ "person_did": "did:key:zMissingContact" })).await;
    assert_shape("contacts_get_missing", &c_get_missing);

    let c_upsert = rpc(
        h,
        "contacts.upsert",
        json!({
            "person_did": "did:key:zContact1",
            "display_name": "Bob",
            "conversation_address": "did:key:zConvBob",
            "favourite": true
        }),
    )
    .await;
    assert_shape("contacts_upsert", &c_upsert);

    let c_get_found = rpc(h, "contacts.get", json!({ "person_did": "did:key:zContact1" })).await;
    assert_shape("contacts_get_found", &c_get_found);

    let c_list_rows = rpc(h, "contacts.list", json!({})).await;
    assert_shape("contacts_list_with_rows", &c_list_rows);

    let c_resolve =
        rpc(h, "contacts.resolve-address", json!({ "person_did": "did:key:zContact1" })).await;
    assert_shape("contacts_resolve_address", &c_resolve);
}

async fn contacts_admit_shapes(h: &Harness) {
    let admit_allow =
        rpc(h, "contacts.admit-first-contact", json!({ "sender_address": "did:key:zAllowSender" }))
            .await;
    assert_shape("contacts_admit_first_contact_allow", &admit_allow);

    rpc(h, "contacts.set-limits", json!({ "window_secs": 3600, "max_per_window": 1 })).await;
    rpc(
        h,
        "contacts.admit-first-contact",
        json!({ "sender_address": "did:key:zRateLimitedSender1" }),
    )
    .await;
    let admit_rl = rpc(
        h,
        "contacts.admit-first-contact",
        json!({ "sender_address": "did:key:zRateLimitedSender2" }),
    )
    .await;
    assert_shape("contacts_admit_first_contact_rate_limited", &admit_rl);

    rpc(h, "contacts.set-limits", json!({ "window_secs": 3600, "max_per_window": 10 })).await;

    let b_list_empty = rpc(h, "block.list", json!({})).await;
    assert_shape("block_list_empty", &b_list_empty);

    let block_add = rpc(
        h,
        "block.add",
        json!({
            "address": "did:key:zBlockedAdmitSender",
            "reason": "spam"
        }),
    )
    .await;
    assert_shape("block_add", &block_add);

    let admit_blocked = rpc(
        h,
        "contacts.admit-first-contact",
        json!({ "sender_address": "did:key:zBlockedAdmitSender" }),
    )
    .await;
    assert_shape("contacts_admit_first_contact_blocked", &admit_blocked);
}

async fn block_shapes(h: &Harness) {
    let b_check_not = rpc(h, "block.check", json!({ "address": "did:key:zNotBlocked" })).await;
    assert_shape("block_check_not_blocked", &b_check_not);

    let b_check_blocked =
        rpc(h, "block.check", json!({ "address": "did:key:zBlockedAdmitSender" })).await;
    assert_shape("block_check_blocked", &b_check_blocked);

    let b_list_rows = rpc(h, "block.list", json!({})).await;
    assert_shape("block_list_with_rows", &b_list_rows);

    let b_remove =
        rpc(h, "block.remove", json!({ "address": "did:key:zBlockedAdmitSender" })).await;
    assert_shape("block_remove", &b_remove);

    let c_remove = rpc(h, "contacts.remove", json!({ "person_did": "did:key:zContact1" })).await;
    assert_shape("contacts_remove", &c_remove);
}

async fn report_shapes(h: &Harness) {
    let r_get_miss = rpc(h, "report.get", json!({ "report_id": "rep_nonexistent" })).await;
    assert_shape("report_get_missing", &r_get_miss);

    let r_list_empty = rpc(h, "report.list", json!({})).await;
    assert_shape("report_list_empty", &r_list_empty);

    let r_create = rpc(
        h,
        "report.create",
        json!({
            "subject_kind": "person",
            "subject_id": "did:key:zBadActor",
            "category": "harassment",
            "details": "Unsolicited spam"
        }),
    )
    .await;
    assert_shape("report_create", &r_create);
    let rid = r_create["report_id"].as_str().unwrap();

    let r_get_found = rpc(h, "report.get", json!({ "report_id": rid })).await;
    assert_shape("report_get_found", &r_get_found);

    let r_list_rows = rpc(h, "report.list", json!({})).await;
    assert_shape("report_list_with_rows", &r_list_rows);

    let r_withdraw = rpc(h, "report.withdraw", json!({ "report_id": rid })).await;
    assert_shape("report_withdraw", &r_withdraw);
}

#[tokio::test]
async fn contacts_block_report_shapes() {
    let h = harness().await;
    contacts_shapes(&h).await;
    contacts_admit_shapes(&h).await;
    block_shapes(&h).await;
    report_shapes(&h).await;
}

async fn conversation_message_shapes(h: &Harness, conv_id: &str) {
    let send_res = rpc(
        h,
        "conversation.send",
        json!({
            "conversation": conv_id,
            "body": "Hello world"
        }),
    )
    .await;
    assert_shape("conversation_send", &send_res);
    let msg_id = send_res["message_id"].as_str().unwrap();

    let d_status = rpc(h, "conversation.delivery-status", json!({ "message_id": msg_id })).await;
    assert_shape("conversation_delivery_status", &d_status);

    let outbox = rpc(h, "conversation.outbox", json!({})).await;
    assert_shape("conversation_outbox", &outbox);

    let changes = rpc(
        h,
        "conversation.changes",
        json!({
            "conversation": conv_id,
            "after_seq": 0,
            "limit": 10
        }),
    )
    .await;
    assert_shape("conversation_changes", &changes);

    let hist_direct = rpc(h, "conversation.history", json!({ "conversation": conv_id })).await;
    assert_shape("conversation_history_direct", &hist_direct);

    let search_res = rpc(h, "conversation.search", json!({ "query": "Hello" })).await;
    assert_shape("conversation_search", &search_res);

    h.notify_state(false, msg_id, ConversationDeliveryState::Failed).await;
    let retry_res = rpc(h, "conversation.retry", json!({ "message_id": msg_id })).await;
    assert_shape("conversation_retry", &retry_res);

    let send_del = rpc(
        h,
        "conversation.send",
        json!({
            "conversation": conv_id,
            "body": "Message to be deleted"
        }),
    )
    .await;
    let del_msg_id = send_del["message_id"].as_str().unwrap();

    let del_res = rpc(
        h,
        "conversation.delete-message",
        json!({
            "message_id": del_msg_id,
            "ask_peer": false
        }),
    )
    .await;
    assert_shape("conversation_delete_message", &del_res);

    let hist_deleted = rpc(h, "conversation.history", json!({ "conversation": conv_id })).await;
    assert_shape("conversation_history_deleted_message", &hist_deleted);
}

#[tokio::test]
async fn conversation_shapes() {
    let h = harness().await;

    let ping = rpc(&h, "conversation.ping", json!({})).await;
    assert_shape("conversation_ping", &ping);

    let (st_unconf, inst, st_conf) = enrol_native_signing(&h, "conversation").await;
    assert_shape("conversation_signing_status_unconfigured", &st_unconf);
    assert_shape("conversation_install_signing_certificate", &inst);
    assert_shape("conversation_signing_status", &st_conf);

    let open_res = rpc(&h, "conversation.open", json!({ "address": "did:key:zPeerDirect" })).await;
    assert_shape("conversation_open", &open_res);
    let conv_id = open_res["conversation_id"].as_str().unwrap().to_string();

    conversation_message_shapes(&h, &conv_id).await;

    let list_direct = rpc(&h, "conversation.list", json!({ "kind": "direct" })).await;
    assert_shape("conversation_list_direct", &list_direct);

    let digest =
        rpc(&h, "conversation.transcript-digest", json!({ "conversation": conv_id })).await;
    assert_shape("conversation_transcript_digest", &digest);

    let export_res = rpc(&h, "conversation.export", json!({})).await;
    assert_shape("conversation_export", &export_res);

    let import_res = rpc(&h, "conversation.import", json!({ "bundle": export_res })).await;
    assert_shape("conversation_import", &import_res);
}

async fn group_lifecycle_shapes(h: &Harness, gid: &str) {
    let grename =
        rpc(h, "group.rename", json!({ "conversation": gid, "name": "Urban Gardeners" })).await;
    assert_shape("group_rename", &grename);

    let conv_svc = did_for_service("conversation");
    let store = h.native_conversation.store_for(&conv_svc).await.unwrap();
    store
        .upsert_session(
            &syneroym_conversation::store::SessionRow {
                peer_address: "did:key:zMemberPeer".to_string(),
                pinned_sig_key: [7u8; 32],
                state: vec![],
            },
            1_000,
        )
        .unwrap();

    let gadd = rpc(
        h,
        "group.add-member",
        json!({ "conversation": gid, "address": "did:key:zMemberPeer" }),
    )
    .await;
    assert_shape("group_add_member", &gadd);

    let gsync = rpc(h, "group.sync", json!({ "conversation": gid })).await;
    assert_shape("group_sync", &gsync);

    let ghide = rpc(h, "group.hide", json!({ "conversation": gid })).await;
    assert_shape("group_hide", &ghide);

    h.deliver(
        false,
        inbound("m-hidden-shape", gid, "did:key:zMemberPeer", 1_000, "hidden message"),
    )
    .await;

    let gunhide = rpc(h, "group.unhide", json!({ "conversation": gid })).await;
    assert_shape("group_unhide", &gunhide);
}

#[tokio::test]
async fn group_shapes() {
    let h = harness().await;
    enrol_native_signing(&h, "profile").await;
    enrol_native_signing(&h, "conversation").await;

    let gcreate = rpc(&h, "group.create", json!({ "name": "Gardeners" })).await;
    assert_shape("group_create", &gcreate);
    let gid = gcreate["conversation_id"].as_str().unwrap().to_string();

    let ginfo_owner = rpc(&h, "group.info", json!({ "conversation": gid })).await;
    assert_shape("group_info_owner", &ginfo_owner);

    group_lifecycle_shapes(&h, &gid).await;

    let ghist = rpc(&h, "conversation.history", json!({ "conversation": gid })).await;
    assert_shape("conversation_history_group_membership", &ghist);

    rpc(&h, "conversation.open", json!({ "address": "did:key:zDirectPeer" })).await;
    let glist = rpc(&h, "conversation.list", json!({})).await;
    assert_shape("conversation_list_with_group", &glist);

    let gremove = rpc(
        &h,
        "group.remove-member",
        json!({ "conversation": gid, "address": "did:key:zMemberPeer" }),
    )
    .await;
    assert_shape("group_remove_member", &gremove);

    let conv_svc = did_for_service("conversation");
    let store = h.native_conversation.store_for(&conv_svc).await.unwrap();
    let gid_copy = gid.clone();
    store
        .queue()
        .transaction(move |tx, _| {
            tx.execute(
                "UPDATE group_members SET removed_epoch = 2 WHERE conversation_id = ?1 AND \
                 member_address = ?2",
                [gid_copy.as_str(), conv_svc.as_str()],
            )?;
            Ok(())
        })
        .unwrap();

    let ginfo_rem = rpc(&h, "group.info", json!({ "conversation": gid })).await;
    assert_shape("group_info_removed_member", &ginfo_rem);
}
