#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! End-to-end integration tests for group chat across three real
//! `syneroym-substrate` nodes.

use std::time::Duration;

use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_conversation::test_support::{clear_clock_offsets, set_clock_offset_ms};
use syneroym_core::config::AppSandboxRole;
use syneroym_roym_core::conversation::group::{
    GROUP_REMOVED_NOTICE, MEMBERSHIP_EVENT_CONTENT_TYPE,
};

mod common;

use common::{
    roym::{roym_artifacts_present, wait_until},
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
    let gid = form_group(&trio.z, "", &[&trio.x, &trio.y]).await;

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
        converge(&[&trio.z, &trio.x, &trio.y], &gid, 3 + (i + 1) * 6).await;

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

    clear_clock_offsets();
    trio.z.teardown().await;
    trio.x.teardown().await;
    trio.y.teardown().await;
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
    let gid = form_group(&trio.z, "", &[&trio.x]).await;

    trio.z.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "early 1" })).await;
    trio.x.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "early 2" })).await;
    converge(&[&trio.z, &trio.x], &gid, 4).await;

    let hist_x = trio.x.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
    let early_ids: Vec<String> = hist_x["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["content_type"] != MEMBERSHIP_EVENT_CONTENT_TYPE)
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();

    let y_conv_did = trio.y.dids["conversation"].clone();
    let y_prof = trio
        .y
        .rpc_ok(
            "profile.set",
            json!({ "display_name": trio.y.label, "conversation_address": y_conv_did }),
        )
        .await;
    let y_env = y_prof["envelope"].as_str().unwrap().to_string();
    let z_conv_did = trio.z.dids["conversation"].clone();
    let z_prof = trio
        .z
        .rpc_ok(
            "profile.set",
            json!({ "display_name": trio.z.label, "conversation_address": z_conv_did }),
        )
        .await;
    let z_env = z_prof["envelope"].as_str().unwrap().to_string();
    trio.z
        .rpc_ok(
            "contacts.upsert",
            json!({ "person_did": trio.y.owner_did(), "profile_envelope": y_env }),
        )
        .await;
    trio.y
        .rpc_ok(
            "contacts.upsert",
            json!({ "person_did": trio.z.owner_did(), "profile_envelope": z_env }),
        )
        .await;

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
    converge(&[&trio.z, &trio.x], &gid, 6).await;

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
        if m["content_type"] != MEMBERSHIP_EVENT_CONTENT_TYPE {
            assert!(m["sender_timestamp_ms"].as_i64().unwrap() >= add_w_ts);
            assert!(!early_ids.contains(&m["id"].as_str().unwrap().to_string()));
        }
    }
    assert!(msgs_w.iter().any(|m| m["body"] == "post-join"));

    trio.z.teardown().await;
    trio.x.teardown().await;
    trio.y.teardown().await;
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
        info["is_member"] == false
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

    trio.z.teardown().await;
    trio.x.teardown().await;
    trio.y.teardown().await;
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

    trio.z.teardown().await;
    trio.x.teardown().await;
    trio.y.teardown().await;
}
