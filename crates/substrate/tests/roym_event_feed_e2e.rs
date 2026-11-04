#![allow(
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    dead_code
)]
use std::{
    path::Path,
    time::{Duration, Instant},
};

use futures::{Stream, StreamExt};
use reqwest::{Client, StatusCode};
use rustls::crypto::ring;
use serde_json::json;
use syneroym_core::protocol_utils::SESSION_COOKIE_NAME;
use syneroym_identity::{Identity, substrate};
use tokio::time;
use tokio_util::bytes::Bytes;

mod common;

use common::roym::{RoymNode, fast_conversation_role, roym_artifacts_present, wait_delivered};

#[derive(Debug, Clone)]
struct SseFrame {
    event: String,
    id: Option<String>,
    data: String,
}

fn parse_sse_frame(frame: &str) -> Option<SseFrame> {
    if frame.starts_with(':') {
        return None;
    }
    let mut event = String::new();
    let mut id = None;
    let mut data = String::new();

    for line in frame.lines() {
        if let Some(rest) = line.strip_prefix("event: ") {
            event = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("id: ") {
            id = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data: ") {
            data = rest.trim().to_string();
        }
    }

    if event.is_empty() {
        return None;
    }

    Some(SseFrame { event, id, data })
}

struct EventStream {
    stream: Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Unpin + Send>,
    buffer: String,
}

impl EventStream {
    async fn next_frame_raw(&mut self) -> Option<String> {
        loop {
            if let Some(idx) = self.buffer.find("\n\n") {
                let frame = self.buffer[..idx].to_string();
                self.buffer.drain(..idx + 2);
                return Some(frame);
            }
            let res = self.stream.next().await?;
            let chunk = res.ok()?;
            let s = String::from_utf8_lossy(&chunk);
            self.buffer.push_str(&s);
        }
    }

    async fn next_event(&mut self) -> SseFrame {
        loop {
            let raw = self.next_frame_raw().await.expect("SSE stream ended unexpectedly");
            if let Some(parsed) = parse_sse_frame(&raw) {
                return parsed;
            }
        }
    }
}

async fn open_event_feed(node: &RoymNode, after: Option<&str>) -> EventStream {
    let client = Client::builder().build().unwrap();
    let mut req = client
        .get(format!("{}/_syneroym/events", node.gateway_url))
        .header("Host", node.web_host_header())
        .header("Accept", "text/event-stream")
        .header(
            "Cookie",
            format!("{SESSION_COOKIE_NAME}={}", node.session_token.as_ref().unwrap()),
        );

    if let Some(cursor) = after {
        req = req.header("Last-Event-ID", cursor);
    }

    let resp = req.send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    EventStream { stream: Box::new(resp.bytes_stream()), buffer: String::new() }
}

async fn setup_pair(temp_a: &Path, temp_b: &Path) -> (RoymNode, RoymNode, String, String) {
    let owner_a = Identity::generate().unwrap();
    let owner_b = Identity::generate().unwrap();

    let mut node_a = RoymNode::boot(
        "node-a",
        temp_a.to_path_buf(),
        None,
        Identity::from_bytes(&owner_a.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    let shared_registry = node_a.registry_url.clone();
    node_a.full_bring_up().await;

    let mut node_b = RoymNode::boot(
        "node-b",
        temp_b.to_path_buf(),
        Some(shared_registry),
        Identity::from_bytes(&owner_b.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    node_b.full_bring_up().await;

    let b_conv_did = node_b.dids["conversation"].clone();
    let a_conv_did = node_a.dids["conversation"].clone();
    let owner_a_did = substrate::derive_did_key(&owner_a.public_key());
    let owner_b_did = substrate::derive_did_key(&owner_b.public_key());

    let a_profile = node_a
        .rpc_ok("profile.set", json!({ "display_name": "Ann", "conversation_address": a_conv_did }))
        .await;
    let b_profile = node_b
        .rpc_ok("profile.set", json!({ "display_name": "Bo", "conversation_address": b_conv_did }))
        .await;

    let a_envelope = a_profile["envelope"].as_str().unwrap().to_string();
    let b_envelope = b_profile["envelope"].as_str().unwrap().to_string();

    node_a
        .rpc_ok(
            "contacts.upsert",
            json!({ "person_did": owner_b_did, "profile_envelope": b_envelope }),
        )
        .await;
    node_b
        .rpc_ok(
            "contacts.upsert",
            json!({ "person_did": owner_a_did, "profile_envelope": a_envelope }),
        )
        .await;

    let opened_a = node_a.rpc_ok("conversation.open", json!({ "person_did": owner_b_did })).await;
    let conv_a = opened_a["conversation_id"].as_str().unwrap().to_string();

    let opened_b = node_b.rpc_ok("conversation.open", json!({ "person_did": owner_a_did })).await;
    let conv_b = opened_b["conversation_id"].as_str().unwrap().to_string();

    (node_a, node_b, conv_a, conv_b)
}

async fn check_inbound_message_reaches_owners_feed_and_changes_shows_it(
    node_a: &RoymNode,
    node_b: &RoymNode,
    conv_a: &str,
    conv_b: &str,
) {
    let mut b_feed = open_event_feed(node_b, None).await;
    let ready = b_feed.next_event().await;
    assert_eq!(ready.event, "ready");

    let sent = node_a
        .rpc_ok("conversation.send", json!({ "conversation": conv_a, "body": "hello B" }))
        .await;
    let message_id = sent["message_id"].as_str().unwrap().to_string();

    let mut found = false;
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        let frame = time::timeout(Duration::from_secs(10), b_feed.next_event())
            .await
            .expect("timeout waiting for frame on B");
        if frame.event == "change" {
            let data: serde_json::Value = serde_json::from_str(&frame.data).unwrap();
            if data["kind"] == "conversation.message"
                && data["subject"] == conv_b
                && data["item"] == message_id
            {
                found = true;
                break;
            }
        }
    }
    assert!(found, "B never received conversation.message change for {message_id}");

    let changes = node_b
        .rpc_ok("conversation.changes", json!({ "conversation": conv_b, "after_seq": 0 }))
        .await;
    let rows = changes["messages"].as_array().expect("array of changes");
    assert!(
        rows.iter().any(|r| r["id"] == message_id),
        "changes on B must immediately contain {message_id}: {rows:?}"
    );
}

async fn check_sent_message_records_a_message_event_within_one_second(
    node_a: &RoymNode,
    conv_a: &str,
) {
    let mut a_feed = open_event_feed(node_a, None).await;
    let ready = a_feed.next_event().await;
    assert_eq!(ready.event, "ready");

    let sent = node_a
        .rpc_ok("conversation.send", json!({ "conversation": conv_a, "body": "speed check" }))
        .await;
    let rpc_return = Instant::now();
    let message_id = sent["message_id"].as_str().unwrap().to_string();

    let mut latency = None;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let frame = time::timeout(Duration::from_secs(2), a_feed.next_event())
            .await
            .expect("timeout waiting for frame on A");
        let arrival = Instant::now();
        if frame.event == "change" {
            let data: serde_json::Value = serde_json::from_str(&frame.data).unwrap();
            if data["kind"] == "conversation.message" && data["item"] == message_id {
                latency = Some(arrival.duration_since(rpc_return));
                break;
            }
        }
    }
    let elapsed = latency.expect("conversation.message event never arrived for sent message");
    assert!(elapsed < Duration::from_secs(1), "event arrived in {elapsed:?}, expected < 1s");
}

async fn check_delivery_events_follow_the_state_to_delivered(node_a: &RoymNode, conv_a: &str) {
    let mut a_feed = open_event_feed(node_a, None).await;
    let ready = a_feed.next_event().await;
    assert_eq!(ready.event, "ready");

    let sent = node_a
        .rpc_ok("conversation.send", json!({ "conversation": conv_a, "body": "deliver me" }))
        .await;
    let message_id = sent["message_id"].as_str().unwrap().to_string();

    let _ = node_a.rpc("conversation.retry", json!({ "message_id": message_id })).await;
    let delivered = wait_delivered(node_a, &message_id).await;
    assert!(delivered, "message failed to reach delivered");

    let mut saw_delivery_event = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let Ok(frame) = time::timeout(Duration::from_millis(500), a_feed.next_event()).await else {
            break;
        };
        if frame.event == "change" {
            let data: serde_json::Value = serde_json::from_str(&frame.data).unwrap();
            if data["kind"] == "conversation.delivery" && data["item"] == message_id {
                saw_delivery_event = true;
                break;
            }
        }
    }
    assert!(saw_delivery_event, "feed should observe conversation.delivery event for {message_id}");
}

async fn check_reconnect_with_last_cursor_gets_missed_events(node_a: &RoymNode, conv_a: &str) {
    let mut a_feed = open_event_feed(node_a, None).await;
    let ready = a_feed.next_event().await;
    assert_eq!(ready.event, "ready");
    let cursor = ready.id.expect("ready frame must have id");
    drop(a_feed);

    let m1 = node_a
        .rpc_ok("conversation.send", json!({ "conversation": conv_a, "body": "missed 1" }))
        .await;
    let m2 = node_a
        .rpc_ok("conversation.send", json!({ "conversation": conv_a, "body": "missed 2" }))
        .await;
    let id1 = m1["message_id"].as_str().unwrap().to_string();
    let id2 = m2["message_id"].as_str().unwrap().to_string();

    let mut resumed_feed = open_event_feed(node_a, Some(&cursor)).await;

    let f1 = resumed_feed.next_event().await;
    assert_eq!(f1.event, "change");
    let d1: serde_json::Value = serde_json::from_str(&f1.data).unwrap();
    assert_eq!(d1["kind"], "conversation.message");
    assert_eq!(d1["item"], id1);

    let f2 = resumed_feed.next_event().await;
    assert_eq!(f2.event, "change");
    let d2: serde_json::Value = serde_json::from_str(&f2.data).unwrap();
    assert_eq!(d2["kind"], "conversation.message");
    assert_eq!(d2["item"], id2);
}

async fn check_cursor_from_before_restart_gets_reload(node_a: &mut RoymNode) {
    let mut a_feed = open_event_feed(node_a, None).await;
    let ready = a_feed.next_event().await;
    assert_eq!(ready.event, "ready");
    let cursor_before_restart = ready.id.expect("ready frame has id");
    drop(a_feed);

    node_a.restart(None, None).await;

    let mut new_feed = open_event_feed(node_a, Some(&cursor_before_restart)).await;
    let first_frame = new_feed.next_event().await;
    assert_eq!(first_frame.event, "reload");
    let data: serde_json::Value = serde_json::from_str(&first_frame.data).unwrap();
    assert!(data.get("cursor").is_some());
}

#[tokio::test]
async fn roym_event_feed_reaches_clients_survives_reconnect_and_reloads_on_restart() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();

    let (mut node_a, node_b, conv_a, conv_b) = setup_pair(dir_a.path(), dir_b.path()).await;

    // 1. Inbound message reaches owner's feed and changes shows it immediately
    check_inbound_message_reaches_owners_feed_and_changes_shows_it(
        &node_a, &node_b, &conv_a, &conv_b,
    )
    .await;

    // 2. Sent message records within 1 second
    check_sent_message_records_a_message_event_within_one_second(&node_a, &conv_a).await;

    // 3. Delivery events follow state
    check_delivery_events_follow_the_state_to_delivered(&node_a, &conv_a).await;

    // 4. Reconnect with last cursor gets missed events
    check_reconnect_with_last_cursor_gets_missed_events(&node_a, &conv_a).await;

    // 5. Cursor from before restart gets reload
    check_cursor_from_before_restart_gets_reload(&mut node_a).await;
}
