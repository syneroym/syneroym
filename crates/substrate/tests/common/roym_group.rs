//! Shared test helpers for Roym group messaging e2e tests.

use std::time::Duration;

use serde_json::{Value, json};
use syneroym_core::config::AppSandboxRole;
use syneroym_identity::Identity;
use tempfile::TempDir;

use super::roym::{CoordinatorNode, RoymNode, fast_conversation_role, wait_until};

pub fn group_role(rekey_secs: u64, max_pending_age_secs: u64) -> AppSandboxRole {
    AppSandboxRole {
        conversation_group_sync_secs: 1,
        conversation_group_rekey_secs: rekey_secs,
        ..fast_conversation_role(max_pending_age_secs)
    }
}

pub struct GroupTrio {
    pub coord: Option<CoordinatorNode>,
    pub z: RoymNode,
    pub x: RoymNode,
    pub y: RoymNode,
    pub dirs: (TempDir, TempDir, TempDir),
}

impl GroupTrio {
    pub async fn teardown(mut self) {
        self.z.teardown().await;
        self.x.teardown().await;
        self.y.teardown().await;
        if let Some(c) = self.coord.take() {
            c.teardown().await;
        }
    }
}

pub async fn boot_trio(role: AppSandboxRole) -> GroupTrio {
    let coord = CoordinatorNode::boot().await;
    let dir_z = tempfile::tempdir().unwrap();
    let dir_x = tempfile::tempdir().unwrap();
    let dir_y = tempfile::tempdir().unwrap();

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
    GroupTrio { coord: Some(coord), z, x, y, dirs: (dir_z, dir_x, dir_y) }
}

pub async fn form_group(owner: &RoymNode, name: &str, members: &[&RoymNode]) -> String {
    let owner_conv_did = owner.dids["conversation"].clone();
    let owner_profile = owner
        .rpc_ok(
            "profile.set",
            json!({ "display_name": owner.label, "conversation_address": owner_conv_did }),
        )
        .await;
    let owner_profile_envelope = owner_profile["envelope"].as_str().unwrap().to_string();
    let owner_did = owner.owner_did();

    for member in members {
        let member_conv_did = member.dids["conversation"].clone();
        let member_profile = member
            .rpc_ok(
                "profile.set",
                json!({ "display_name": member.label, "conversation_address": member_conv_did }),
            )
            .await;
        let member_profile_envelope = member_profile["envelope"].as_str().unwrap().to_string();
        let member_did = member.owner_did();

        owner
            .rpc_ok(
                "contacts.upsert",
                json!({ "person_did": &member_did, "profile_envelope": member_profile_envelope }),
            )
            .await;
        member
            .rpc_ok(
                "contacts.upsert",
                json!({ "person_did": &owner_did, "profile_envelope": &owner_profile_envelope }),
            )
            .await;
    }

    let created = owner.rpc_ok("group.create", json!({})).await;
    let gid = created["conversation_id"].as_str().unwrap().to_string();

    for member in members {
        owner
            .rpc_ok(
                "group.add-member",
                json!({ "conversation": &gid, "person_did": member.owner_did() }),
            )
            .await;
    }

    if !name.is_empty() {
        owner.rpc_ok("group.rename", json!({ "conversation": &gid, "name": name })).await;
    }

    for member in members {
        let ok = wait_until(Duration::from_secs(60), || async {
            let list = member.rpc_ok("conversation.list", json!({ "kind": "group" })).await;
            list["conversations"].as_array().is_some_and(|rows| {
                rows.iter().any(|r| {
                    r["id"] == gid
                        && (name.is_empty()
                            || r.get("group").and_then(|g| g.get("name")).and_then(Value::as_str)
                                == Some(name))
                })
            })
        })
        .await;
        assert!(ok, "{} did not adopt group {} in time", member.label, gid);
    }

    if !name.is_empty() {
        let list_owner = owner.rpc_ok("conversation.list", json!({ "kind": "group" })).await;
        let owner_count = list_owner["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == gid)
            .and_then(|r| r["message_count"].as_i64())
            .expect("owner group row message count");
        for member in members {
            let list_m = member.rpc_ok("conversation.list", json!({ "kind": "group" })).await;
            let m_count = list_m["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["id"] == gid)
                .and_then(|r| r["message_count"].as_i64())
                .expect("member group row message count");
            assert_eq!(
                owner_count, m_count,
                "owner message count differs from member {}",
                member.label
            );
        }
    }

    gid
}

pub async fn projection(node: &RoymNode, gid: &str) -> Vec<Value> {
    let hist = node.rpc_ok("conversation.history", json!({ "conversation": gid })).await;
    hist["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|m| {
            json!({
                "id": m["id"],
                "author": m["author"],
                "sender_timestamp_ms": m["sender_timestamp_ms"],
                "content_type": m["content_type"],
                "body": m["body"],
            })
        })
        .collect()
}

pub async fn digest(node: &RoymNode, gid: &str) -> String {
    let res = node.rpc_ok("conversation.transcript-digest", json!({ "conversation": gid })).await;
    res["digest"].as_str().unwrap().to_string()
}

pub async fn converge(nodes: &[&RoymNode], gid: &str, expect_rows: usize) {
    let last_diag = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let last_diag_ref = last_diag.clone();
    let ok = wait_until(Duration::from_secs(60), || {
        let last_diag = last_diag_ref.clone();
        async move {
            for node in nodes {
                let _ = node.rpc("group.sync", json!({ "conversation": gid })).await;
            }
            let mut digests = Vec::with_capacity(nodes.len());
            let mut diag = Vec::new();
            for node in nodes {
                let hist =
                    node.rpc_ok("conversation.history", json!({ "conversation": gid })).await;
                let rows = hist["messages"].as_array().map_or(0, Vec::len);
                let d = digest(node, gid).await;
                let short_d = if d.len() >= 8 { &d[..8] } else { &d };
                diag.push(format!(
                    "{}: {}/{} rows, digest={}",
                    node.label, rows, expect_rows, short_d
                ));
                if rows == expect_rows {
                    digests.push(d);
                }
            }
            let summary = diag.join("; ");
            if let Ok(mut lock) = last_diag.lock() {
                *lock = summary;
            }
            digests.len() == nodes.len() && digests.windows(2).all(|w| w[0] == w[1])
        }
    })
    .await;
    let diag_str = last_diag.lock().map_or_else(|_| String::new(), |l| l.clone());
    assert!(ok, "nodes failed to converge on group {gid} within 60s: {diag_str}");
}
