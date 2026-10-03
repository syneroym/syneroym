#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! End-to-end integration tests for group chat across three real
//! `syneroym-substrate` nodes.

use std::time::Duration;

use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_conversation::test_support::{clear_clock_offsets, set_clock_offset_ms};
use syneroym_core::config::AppSandboxRole;
use syneroym_roym_core::conversation::group::{
    GROUP_PROFILE_CONTENT_TYPE, GROUP_REMOVED_NOTICE, MEMBERSHIP_EVENT_CONTENT_TYPE,
    parse_group_profile,
};

mod common;

use common::{
    roym::{RoymNode, roym_artifacts_present, wait_until},
    roym_group::{boot_trio, converge, digest, form_group, group_role, projection},
};

#[tokio::test]
async fn three_members_see_one_order_from_skewed_clocks() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let role = group_role(3600, AppSandboxRole::default().conversation_max_pending_age_secs);
    let trio = boot_trio(role).await;
    let gid = form_group(&trio.z, "Skew Group", &[&trio.x, &trio.y]).await;

    set_clock_offset_ms(&trio.x.dids["conversation"], 90_000);
    set_clock_offset_ms(&trio.y.dids["conversation"], -90_000);
    set_clock_offset_ms(&trio.z.dids["conversation"], 0);

    for i in 0..5 {
        let sends = vec![
            trio.z.rpc(
                "conversation.send",
                json!({ "conversation": &gid, "body": format!("z-{i}-1") }),
            ),
            trio.z.rpc(
                "conversation.send",
                json!({ "conversation": &gid, "body": format!("z-{i}-2") }),
            ),
            trio.x.rpc(
                "conversation.send",
                json!({ "conversation": &gid, "body": format!("x-{i}-1") }),
            ),
            trio.x.rpc(
                "conversation.send",
                json!({ "conversation": &gid, "body": format!("x-{i}-2") }),
            ),
            trio.y.rpc(
                "conversation.send",
                json!({ "conversation": &gid, "body": format!("y-{i}-1") }),
            ),
            trio.y.rpc(
                "conversation.send",
                json!({ "conversation": &gid, "body": format!("y-{i}-2") }),
            ),
        ];
        let send_res = futures::future::join_all(sends).await;
        for (idx, r) in send_res.iter().enumerate() {
            assert!(r.get("error").is_none(), "send {idx} in iteration {i} failed: {r:?}");
        }
        converge(&[&trio.z, &trio.x, &trio.y], &gid, 4 + (i + 1) * 6).await;

        let pz = projection(&trio.z, &gid).await;
        let px = projection(&trio.x, &gid).await;
        let py = projection(&trio.y, &gid).await;
        assert_eq!(serde_json::to_vec(&pz).unwrap(), serde_json::to_vec(&px).unwrap());
        assert_eq!(serde_json::to_vec(&pz).unwrap(), serde_json::to_vec(&py).unwrap());

        let mut sorted = pz.clone();
        sorted.sort_by(|a, b| {
            let ts_a = a["sender_timestamp_ms"].as_i64().unwrap();
            let ts_b = b["sender_timestamp_ms"].as_i64().unwrap();
            let auth_a = a["author"].as_str().unwrap();
            let auth_b = b["author"].as_str().unwrap();
            let id_a = a["id"].as_str().unwrap();
            let id_b = b["id"].as_str().unwrap();
            (ts_a, auth_a, id_a).cmp(&(ts_b, auth_b, id_b))
        });
        assert_eq!(pz, sorted);

        let dz = digest(&trio.z, &gid).await;
        assert_eq!(dz, digest(&trio.x, &gid).await);
        assert_eq!(dz, digest(&trio.y, &gid).await);
    }

    for node in [&trio.z, &trio.x, &trio.y] {
        let info = node.rpc_ok("group.info", json!({ "conversation": &gid })).await;
        assert_eq!(info["name"], "Skew Group", "group name on {}", node.label);
        let hist = node.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
        let profile_rows: Vec<_> = hist["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["content_type"] == GROUP_PROFILE_CONTENT_TYPE)
            .collect();
        assert_eq!(profile_rows.len(), 1, "profile row count on {}", node.label);
        let profile_body = profile_rows[0]["body"].as_str().unwrap();
        assert_eq!(parse_group_profile(profile_body.as_bytes()).unwrap(), "Skew Group");
    }

    clear_clock_offsets();
    trio.teardown().await;
}

async fn exchange_contacts(a: &RoymNode, b: &RoymNode) {
    let a_conv = a.dids["conversation"].clone();
    let a_prof = a
        .rpc_ok("profile.set", json!({ "display_name": a.label, "conversation_address": a_conv }))
        .await;
    let a_env = a_prof["envelope"].as_str().unwrap().to_string();
    let b_conv = b.dids["conversation"].clone();
    let b_prof = b
        .rpc_ok("profile.set", json!({ "display_name": b.label, "conversation_address": b_conv }))
        .await;
    let b_env = b_prof["envelope"].as_str().unwrap().to_string();
    a.rpc_ok("contacts.upsert", json!({ "person_did": b.owner_did(), "profile_envelope": b_env }))
        .await;
    b.rpc_ok("contacts.upsert", json!({ "person_did": a.owner_did(), "profile_envelope": a_env }))
        .await;
}

async fn membership_event_rows(node: &RoymNode, gid: &str) -> Vec<(String, String)> {
    let hist = node.rpc_ok("conversation.history", json!({ "conversation": gid })).await;
    hist["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["content_type"] == MEMBERSHIP_EVENT_CONTENT_TYPE)
        .map(|m| (m["id"].as_str().unwrap().to_string(), m["body"].as_str().unwrap().to_string()))
        .collect()
}

#[tokio::test]
async fn a_joiner_reads_nothing_before_joining() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let role = group_role(3600, AppSandboxRole::default().conversation_max_pending_age_secs);
    let trio = boot_trio(role).await;
    let gid = form_group(&trio.z, "Joiner Group", &[&trio.x]).await;

    trio.z.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "early 1" })).await;
    trio.x.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "early 2" })).await;
    converge(&[&trio.z, &trio.x], &gid, 5).await;

    let hist_x = trio.x.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
    let early_ids: Vec<String> = hist_x["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| {
            m["content_type"] != MEMBERSHIP_EVENT_CONTENT_TYPE
                && m["content_type"] != GROUP_PROFILE_CONTENT_TYPE
        })
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();

    exchange_contacts(&trio.y, &trio.z).await;

    trio.z
        .rpc_ok("group.add-member", json!({ "group": &gid, "person_did": trio.y.owner_did() }))
        .await;
    let ok = wait_until(Duration::from_secs(60), || async {
        let list = trio.y.rpc_ok("conversation.list", json!({ "kind": "group" })).await;
        list["conversations"].as_array().is_some_and(|rows| rows.iter().any(|r| r["id"] == gid))
    })
    .await;
    assert!(ok);

    trio.z.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "post-join" })).await;
    // Rows: the name entry, three membership events (creation, x, y), the two
    // early messages and the post-join message. Adding a member no longer
    // posts the name again as a chat message.
    converge(&[&trio.z, &trio.x], &gid, 7).await;

    let ok_w = wait_until(Duration::from_secs(60), || async {
        let _ = trio.y.rpc("group.sync", json!({ "group": &gid })).await;
        let hist = trio.y.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
        let msgs = hist["messages"].as_array().cloned().unwrap_or_default();
        msgs.iter().any(|m| m["body"] == "post-join")
    })
    .await;
    assert!(ok_w);

    let hist_w = trio.y.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
    let msgs_w = hist_w["messages"].as_array().unwrap();
    let add_w_event = msgs_w
        .iter()
        .find(|m| {
            m["content_type"] == MEMBERSHIP_EVENT_CONTENT_TYPE
                && m["body"].as_str().is_some_and(|b| b.contains(&trio.y.dids["conversation"]))
        })
        .expect("add W event found in W");
    let add_w_ts = add_w_event["sender_timestamp_ms"].as_i64().unwrap();
    for m in msgs_w {
        if m["content_type"] != MEMBERSHIP_EVENT_CONTENT_TYPE
            && m["content_type"] != GROUP_PROFILE_CONTENT_TYPE
        {
            assert!(m["sender_timestamp_ms"].as_i64().unwrap() >= add_w_ts);
            assert!(!early_ids.contains(&m["id"].as_str().unwrap().to_string()));
        }
    }
    assert!(msgs_w.iter().any(|m| m["body"] == "post-join"));

    for node in [&trio.z, &trio.x, &trio.y] {
        let info = node.rpc_ok("group.info", json!({ "conversation": &gid })).await;
        assert_eq!(info["name"], "Joiner Group", "group name on {}", node.label);
    }

    let z_members = membership_event_rows(&trio.z, &gid).await;
    let x_members = membership_event_rows(&trio.x, &gid).await;
    let y_members = membership_event_rows(&trio.y, &gid).await;
    assert_eq!(z_members, x_members, "membership rows between Z and X");
    assert_eq!(z_members, y_members, "membership rows between Z and Y");

    trio.teardown().await;
}

#[tokio::test]
async fn a_removed_member_reads_nothing_after_removal() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let role = group_role(3600, AppSandboxRole::default().conversation_max_pending_age_secs);
    let mut trio = boot_trio(role).await;
    let gid = form_group(&trio.z, "", &[&trio.x, &trio.y]).await;

    let m_resp =
        trio.y.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "msg M" })).await;
    let m_id = m_resp["message_id"].as_str().unwrap().to_string();

    let ok_m = wait_until(Duration::from_secs(30), || async {
        let s = trio.y.rpc_ok("conversation.delivery-status", json!({ "message_id": &m_id })).await;
        s["state"] == "delivered"
    })
    .await;
    assert!(ok_m, "msg M was not delivered before Y was stopped");

    trio.y.stop(None).await;

    let p_resp =
        trio.x.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "msg P" })).await;
    let p_id = p_resp["message_id"].as_str().unwrap().to_string();

    let ok = wait_until(Duration::from_secs(60), || async {
        let hz = trio.z.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
        hz["messages"].as_array().is_some_and(|rows| rows.iter().any(|r| r["id"] == p_id))
    })
    .await;
    assert!(ok);

    trio.z
        .rpc_ok("group.remove-member", json!({ "group": &gid, "person_did": trio.y.owner_did() }))
        .await;
    trio.z
        .rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "after removal" }))
        .await;
    converge(&[&trio.z, &trio.x], &gid, 7).await;

    trio.y.resume(None).await;
    trio.y.republish_registry().await;
    trio.y.login().await;

    let ok = wait_until(Duration::from_secs(60), || async {
        let _ = trio.y.rpc("group.sync", json!({ "group": &gid })).await;
        let info = trio.y.rpc_ok("group.info", json!({ "conversation": &gid })).await;
        let hy = trio.y.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
        info["is_member"] == false
            && hy["messages"].as_array().is_some_and(|m| m.iter().any(|r| r["id"] == p_id))
    })
    .await;
    assert!(ok);

    let hy = trio.y.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
    let msgs_y = hy["messages"].as_array().unwrap();
    assert!(msgs_y.iter().any(|m| m["id"] == p_id));
    assert!(!msgs_y.iter().any(|m| m["body"] == "after removal"));

    let iy = trio.y.rpc_ok("group.info", json!({ "conversation": &gid })).await;
    assert!(iy["key_epoch"].as_u64().unwrap() < iy["epoch"].as_u64().unwrap());
    assert_eq!(iy["notices"]["removed"], GROUP_REMOVED_NOTICE);

    let send_err =
        trio.y.rpc_err("conversation.send", json!({ "conversation": &gid, "body": "fail" })).await;
    assert_eq!(send_err["message"], GROUP_REMOVED_NOTICE);
    let ob = trio.y.rpc_ok("conversation.outbox", json!({})).await;
    assert_eq!(ob["outbox"].as_array().unwrap().len(), 0);

    let del_y = trio.y.rpc_ok("conversation.delete-message", json!({ "message_id": m_id })).await;
    assert_eq!(del_y["asked_peer"], false);
    assert!(del_y["note"].as_str().unwrap().contains("nobody else in this group can receive one"));

    let del_x = trio
        .x
        .rpc_ok("conversation.delete-message", json!({ "message_id": p_id, "ask_peer": true }))
        .await;
    assert_eq!(del_x["asked_peer"], true);
    assert!(del_x["note"].as_str().unwrap().contains("A request to delete it was sent"));

    converge(&[&trio.z, &trio.x], &gid, 7).await;
    let ok_tomb = wait_until(Duration::from_secs(60), || async {
        let hz = trio.z.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
        hz["messages"].as_array().is_some_and(|rows| {
            rows.iter().any(|r| {
                r["id"] == p_id && r.get("deleted_at_secs").and_then(Value::as_i64).is_some()
            })
        })
    })
    .await;
    assert!(ok_tomb);

    trio.teardown().await;
}

#[tokio::test]
async fn a_scheduled_rekey_changes_the_key_with_stable_membership() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let role = group_role(5, AppSandboxRole::default().conversation_max_pending_age_secs);
    let trio = boot_trio(role).await;
    let gid = form_group(&trio.z, "", &[&trio.x, &trio.y]).await;

    let init_epoch = trio.z.rpc_ok("group.info", json!({ "conversation": &gid })).await["epoch"]
        .as_u64()
        .unwrap();

    let ok = wait_until(Duration::from_secs(60), || async {
        let _ = trio.z.rpc("group.sync", json!({ "group": &gid })).await;
        let _ = trio.x.rpc("group.sync", json!({ "group": &gid })).await;
        let _ = trio.y.rpc("group.sync", json!({ "group": &gid })).await;
        let iz = trio.z.rpc_ok("group.info", json!({ "conversation": &gid })).await;
        let ix = trio.x.rpc_ok("group.info", json!({ "conversation": &gid })).await;
        let iy = trio.y.rpc_ok("group.info", json!({ "conversation": &gid })).await;
        let ez = iz["epoch"].as_u64().unwrap_or(0);
        let kx = ix["key_epoch"].as_u64().unwrap_or(0);
        let ky = iy["key_epoch"].as_u64().unwrap_or(0);
        ez > init_epoch && kx == ez && ky == ez
    })
    .await;
    assert!(ok);

    trio.z.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "post-rekey" })).await;
    converge(&[&trio.z, &trio.x, &trio.y], &gid, 4).await;

    let hz_post = trio.z.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
    let hx_post = trio.x.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
    let hy_post = trio.y.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
    for (label, hist) in [("z", &hz_post), ("x", &hx_post), ("y", &hy_post)] {
        let mem_count = hist["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["content_type"] == MEMBERSHIP_EVENT_CONTENT_TYPE)
            .count();
        assert_eq!(mem_count, 3, "membership event count on {label} after rekey");
    }

    trio.teardown().await;
}
