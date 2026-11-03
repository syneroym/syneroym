use serde_json::json;
use syneroym_core::event_bus::{Cursor, ReadOutcome};

use super::{fixtures::*, helpers::*};

#[tokio::test]
async fn sending_a_message_records_the_same_event_in_both_builds() {
    let h = harness().await;
    let conv_id = open_conv(&h, "did:key:zPeerEvents1").await;
    assert!(!conv_id.is_empty());

    let (sw, sn) = both_rpc(
        &h,
        "conversation.send",
        json!({ "conversation": conv_id, "body": "hello parity events" }),
    )
    .await;
    let wasm_msg_id = sw["result"]["message_id"].as_str().unwrap();
    let native_msg_id = sn["result"]["message_id"].as_str().unwrap();

    let now =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()
            as i64;
    let wasm_start = Cursor { run_id: h.wasm_event_bus.run_id().to_string(), seq: 0 };
    let native_start = Cursor { run_id: h.native_event_bus.run_id().to_string(), seq: 0 };

    let wasm_outcome = h.wasm_event_bus.read("roym", Some(&wasm_start), 100, now);
    let native_outcome = h.native_event_bus.read("roym", Some(&native_start), 100, now);

    let wasm_events = match wasm_outcome {
        ReadOutcome::Events { events, .. } => events,
        ReadOutcome::Reload { .. } => Vec::new(),
    };
    let native_events = match native_outcome {
        ReadOutcome::Events { events, .. } => events,
        ReadOutcome::Reload { .. } => Vec::new(),
    };

    assert_eq!(wasm_events.len(), 1);
    assert_eq!(native_events.len(), 1);

    assert_eq!(wasm_events[0].kind, "conversation.message");
    assert_eq!(native_events[0].kind, "conversation.message");

    assert_eq!(wasm_events[0].service, "conversation");
    assert_eq!(native_events[0].service, "conversation");

    assert_eq!(wasm_events[0].subject, conv_id);
    assert_eq!(native_events[0].subject, conv_id);

    assert_eq!(wasm_events[0].item.as_deref(), Some(wasm_msg_id));
    assert_eq!(native_events[0].item.as_deref(), Some(native_msg_id));
}
