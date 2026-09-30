#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Offline and edge-case end-to-end integration tests for group messaging.

use std::{fs, time::Duration};

use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_core::{config::AppSandboxRole, dht_registry::RegistryClient};
use syneroym_identity::Identity;
use syneroym_roym_core::clock;
use tokio::time;

mod common;

use common::{
    roym::{CoordinatorNode, RoymNode, roym_artifacts_present, wait_until},
    roym_group::{boot_trio, converge, digest, form_group, group_role},
};

#[tokio::test]
async fn an_offline_member_pulls_the_gap_from_another_member() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let role = group_role(3600, AppSandboxRole::default().conversation_max_pending_age_secs);
    let mut trio = boot_trio(role).await;
    let gid = form_group(&trio.z, "", &[&trio.x, &trio.y]).await;

    trio.y.stop(None).await;

    trio.x.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "x-1" })).await;
    trio.x.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "x-2" })).await;
    trio.z.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "z-1" })).await;
    trio.z.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "z-2" })).await;

    let ok = wait_until(Duration::from_secs(120), || async {
        let _ = trio.x.rpc("group.sync", json!({ "group": &gid })).await;
        let hx = trio.x.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
        let msgs = hx["messages"].as_array().cloned().unwrap_or_default();
        ["x-1", "x-2", "z-1", "z-2"].iter().all(|b| msgs.iter().any(|m| m["body"] == *b))
    })
    .await;
    assert!(ok, "X did not receive all 4 messages");

    trio.z.stop(None).await;

    trio.y.resume(None).await;
    trio.y.republish_registry().await;
    trio.y.login().await;

    let ok_y = wait_until(Duration::from_secs(120), || async {
        let _ = trio.y.rpc("group.sync", json!({ "group": &gid })).await;
        let dy = digest(&trio.y, &gid).await;
        let dx = digest(&trio.x, &gid).await;
        dy == dx
    })
    .await;
    assert!(ok_y, "Y digest did not equal X digest while Z was offline");

    let hy = trio.y.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
    let msgs_y = hy["messages"].as_array().unwrap();
    for body in ["x-1", "x-2", "z-1", "z-2"] {
        assert!(msgs_y.iter().any(|m| m["body"] == body));
    }

    trio.teardown().await;
}

#[tokio::test]
async fn no_member_to_member_message_passes_through_non_members_storage() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let coord = CoordinatorNode::boot().await;
    let dir_z = tempfile::tempdir().unwrap();
    let dir_x = tempfile::tempdir().unwrap();
    let dir_y = tempfile::tempdir().unwrap();
    let role = group_role(3600, AppSandboxRole::default().conversation_max_pending_age_secs);

    let z = RoymNode::boot_on(
        "node-z",
        dir_z.path().to_path_buf(),
        &coord,
        Identity::generate().unwrap(),
        role.clone(),
    )
    .await;
    let x = RoymNode::boot_on(
        "node-x",
        dir_x.path().to_path_buf(),
        &coord,
        Identity::generate().unwrap(),
        role.clone(),
    )
    .await;
    let y = RoymNode::boot_on(
        "node-y",
        dir_y.path().to_path_buf(),
        &coord,
        Identity::generate().unwrap(),
        role,
    )
    .await;

    let gid = form_group(&z, "", &[&x, &y]).await;

    z.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "round 1 from z" })).await;
    x.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "round 1 from x" })).await;
    y.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "round 1 from y" })).await;

    converge(&[&z, &x, &y], &gid, 6).await;

    let dz = digest(&z, &gid).await;
    assert_eq!(dz, digest(&x, &gid).await);
    assert_eq!(dz, digest(&y, &gid).await);

    let info_z = z.rpc_ok("group.info", json!({ "conversation": &gid })).await;
    let members = info_z["members"].as_array().unwrap();
    assert_eq!(members.len(), 3);
    let mut member_addrs: Vec<&str> =
        members.iter().map(|m| m["address"].as_str().expect("address")).collect();
    member_addrs.sort_unstable();
    let mut expected = vec![
        z.dids["conversation"].as_str(),
        x.dids["conversation"].as_str(),
        y.dids["conversation"].as_str(),
    ];
    expected.sort_unstable();
    assert_eq!(member_addrs, expected);

    assert_coordinator_has_no_conversation_state(&coord, &[&z, &x, &y], &dir_z).await;

    z.teardown().await;
    x.teardown().await;
    y.teardown().await;
    coord.teardown().await;
}

async fn assert_coordinator_has_no_conversation_state(
    coord: &CoordinatorNode,
    members: &[&RoymNode],
    dir_z: &tempfile::TempDir,
) {
    let reg = RegistryClient::new(false, Some(coord.registry_url().to_string()));
    let coord_node = reg.lookup(coord.0.did(), false).await;
    assert!(coord_node.is_ok(), "coordinator node itself is registered");

    for m in members {
        let conv_did = &m.dids["conversation"];
        let rec = reg.lookup(conv_did, false).await.expect("member conversation registered");
        assert_eq!(rec.info.substrate_id, m.substrate_did());
        assert_ne!(rec.info.substrate_id, coord.0.did());
    }

    let coord_svcs = coord.0.substrate_client.list_svcs().await.expect("list coord svcs");
    assert!(
        !coord_svcs.iter().any(|s| s.interfaces.iter().any(|i| i == "conversation")),
        "coordinator has no conversation service deployed"
    );

    let z_conv_db = dir_z
        .path()
        .join("data")
        .join("db")
        .join("services")
        .join(&members[0].dids["conversation"])
        .join("conversation.db");
    assert!(z_conv_db.exists(), "member node z must host its conversation.db");

    let coord_services_dir = coord.0.base_path().join("data").join("db").join("services");
    if coord_services_dir.exists() {
        for entry in fs::read_dir(&coord_services_dir).unwrap().flatten() {
            assert!(
                !entry.path().join("conversation.db").exists(),
                "coordinator must hold no conversation database at {:?}",
                entry.path()
            );
        }
    }
}

#[tokio::test]
async fn a_stranger_adding_you_is_a_first_contact() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let dir_x = tempfile::tempdir().unwrap();
    let role = group_role(3600, AppSandboxRole::default().conversation_max_pending_age_secs);
    let x = RoymNode::boot_ready(
        "node-x",
        dir_x.path().to_path_buf(),
        None,
        Identity::generate().unwrap(),
        role.clone(),
    )
    .await;
    let reg = Some(x.registry_url.clone());

    let dir_w = tempfile::tempdir().unwrap();
    let w = RoymNode::boot_ready(
        "node-w",
        dir_w.path().to_path_buf(),
        reg,
        Identity::generate().unwrap(),
        role,
    )
    .await;

    x.rpc_ok("contacts.set-limits", json!({ "window_secs": 3600, "max_per_window": 0 })).await;

    let created = w.rpc_ok("group.create", json!({ "name": "Stranger Group" })).await;
    let gid = created["conversation_id"].as_str().unwrap().to_string();
    w.rpc_ok("group.add-member", json!({ "group": &gid, "address": x.dids["conversation"] })).await;

    w.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "msg A" })).await;

    let ok = wait_until(Duration::from_secs(60), || async {
        let list = x.rpc_ok("conversation.list", json!({ "kind": "group" })).await;
        if !list["conversations"].as_array().unwrap().is_empty() {
            return false;
        }
        let hidden =
            x.rpc_ok("conversation.list", json!({ "kind": "group", "include_hidden": true })).await;
        hidden["conversations"].as_array().is_some_and(|rows| {
            rows.iter().any(|r| {
                r["id"] == gid
                    && r.get("group")
                        .and_then(|g| g.get("admission"))
                        .and_then(|a| a.get("state"))
                        .and_then(Value::as_str)
                        == Some("refused")
            })
        })
    })
    .await;
    assert!(ok);

    let unh = x.rpc_ok("group.unhide", json!({ "group": &gid })).await;
    assert_eq!(unh["filled_in"], 2);

    let info_x = x.rpc_ok("group.info", json!({ "conversation": &gid })).await;
    assert_eq!(info_x["name"], "Stranger Group");

    let list_x = x.rpc_ok("conversation.list", json!({ "kind": "group" })).await;
    let row_x = list_x["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == gid)
        .expect("unhidden group row");
    assert_eq!(row_x["message_count"], 2);
    assert_eq!(row_x["group"]["name"], "Stranger Group");

    let hx = x.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
    assert!(hx["messages"].as_array().unwrap().iter().any(|m| m["body"] == "msg A"));

    w.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "msg B" })).await;
    let ok_b = wait_until(Duration::from_secs(60), || async {
        let _ = x.rpc("group.sync", json!({ "group": &gid })).await;
        let hx = x.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
        hx["messages"].as_array().is_some_and(|rows| rows.iter().any(|m| m["body"] == "msg B"))
    })
    .await;
    assert!(ok_b);

    x.teardown().await;
    w.teardown().await;
}

#[tokio::test]
async fn a_message_to_a_member_removed_while_pending_settles_failed_after_the_age_window() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let mut trio = boot_trio(group_role(3600, 60)).await;
    let gid = form_group(&trio.z, "", &[&trio.x, &trio.y]).await;

    trio.y.stop(None).await;

    let m_resp =
        trio.z.rpc_ok("conversation.send", json!({ "conversation": &gid, "body": "msg M" })).await;
    let m_id = m_resp["message_id"].as_str().unwrap().to_string();
    let m_ts = m_resp["sender_timestamp_ms"].as_i64().unwrap();

    let ok_x = wait_until(Duration::from_secs(60), || async {
        let hx = trio.x.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
        hx["messages"].as_array().is_some_and(|rows| rows.iter().any(|r| r["id"] == m_id))
    })
    .await;
    assert!(ok_x);

    trio.z
        .rpc_ok("group.remove-member", json!({ "group": &gid, "person_did": trio.y.owner_did() }))
        .await;

    let start_now = clock::now_ms();
    assert!(
        start_now <= m_ts + 30_000,
        "removal step overran the 30s observation window (start_now={start_now}, m_ts={m_ts})"
    );
    let target_20 = m_ts + 20_000;
    if target_20 > start_now {
        time::sleep(Duration::from_millis((target_20 - start_now) as u64)).await;
    }
    let hz_20 = trio.z.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
    let row_20 = hz_20["messages"].as_array().unwrap().iter().find(|m| m["id"] == m_id).unwrap();
    assert_eq!(row_20["state"], "pending");

    let now_30 = clock::now_ms();
    let target_30 = m_ts + 30_000;
    if target_30 > now_30 {
        time::sleep(Duration::from_millis((target_30 - now_30) as u64)).await;
    }
    let hz_30 = trio.z.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
    let row_30 = hz_30["messages"].as_array().unwrap().iter().find(|m| m["id"] == m_id).unwrap();
    assert_eq!(row_30["state"], "pending");

    let iz = trio.z.rpc_ok("group.info", json!({ "conversation": &gid })).await;
    let members = iz["members"].as_array().unwrap();
    assert!(!members.iter().any(|m| m["address"] == trio.y.dids["conversation"]));

    let ok_fail = wait_until(Duration::from_secs(45), || async {
        let hz = trio.z.rpc_ok("conversation.history", json!({ "conversation": &gid })).await;
        hz["messages"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|r| r["id"] == m_id && r["state"] == "failed"))
    })
    .await;
    assert!(ok_fail);
    let now_failed = clock::now_ms();
    assert!(now_failed >= m_ts + 60_000, "message settled to failed before 60s age window passed");

    trio.teardown().await;
}
