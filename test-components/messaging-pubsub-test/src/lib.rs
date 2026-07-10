#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Messaging pub/sub test guest component
//!
//! Exercises `syneroym:messaging/host-api` (`subscribe`/`publish`) and the
//! optional `syneroym:messaging/guest-api::handle-message` push-delivery
//! export end-to-end for M3B Slice 6A integration tests.

use bindings::{
    Guest,
    exports::{
        syneroym::messaging::guest_api::Guest as GuestApiGuest,
        syneroym_test::messaging_pubsub_test::test_driver::Guest as TestDriverGuest,
    },
    syneroym::{
        data_layer::store::{self, CollectionSchema, QueryOptions, RecordWriteValue},
        messaging::host_api,
    },
};

mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "messaging-pubsub-test",
        with: {
            "syneroym:messaging/host-api@0.1.0": generate,
            "syneroym:data-layer/store@0.1.0": generate,
        },
    });

    use super::MessagingPubsubTestComponent;
    export!(MessagingPubsubTestComponent);
}

const RECEIVED_MESSAGES_COLLECTION: &str = "received_messages";

/// `syneroym:data-layer/store::put` requires the record payload to be valid
/// JSON (it validates this at the host boundary), so `(topic, payload)` is
/// JSON-encoded rather than packed into a raw byte string. `payload` is
/// stored as UTF-8 text (lossy-converted if not) -- this test fixture only
/// ever sends UTF-8 text payloads (see `publish-to`'s `payload: string`
/// parameter); the real `guest-api::handle-message` contract's `list<u8>`
/// payload is untouched.
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredMessage {
    topic: String,
    payload: String,
}

fn encode_record(topic: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
    let message = StoredMessage {
        topic: topic.to_string(),
        payload: String::from_utf8_lossy(payload).into_owned(),
    };
    serde_json::to_vec(&message).map_err(|e| format!("failed to encode record: {e}"))
}

fn decode_record(bytes: &[u8]) -> Result<(String, Vec<u8>), String> {
    let message: StoredMessage =
        serde_json::from_slice(bytes).map_err(|e| format!("failed to decode record: {e}"))?;
    Ok((message.topic, message.payload.into_bytes()))
}

struct MessagingPubsubTestComponent;

impl Guest for MessagingPubsubTestComponent {
    fn init() -> Result<(), String> {
        store::create_collection(&CollectionSchema {
            name: RECEIVED_MESSAGES_COLLECTION.to_string(),
            indexes: vec![],
        })
        .map_err(|e| format!("{e:?}"))
    }

    fn migrate() -> Result<(), String> {
        // No schema changes needed across re-deploys of this fixture.
        Ok(())
    }
}

impl GuestApiGuest for MessagingPubsubTestComponent {
    fn handle_message(topic: String, payload: Vec<u8>) -> Result<(), String> {
        // Every host invocation gets a fresh Store/instance, so the count of
        // already-received messages must be read back from the host-durable
        // data-layer collection rather than an in-guest counter.
        let existing = store::query(
            RECEIVED_MESSAGES_COLLECTION,
            &QueryOptions { filter: None, limit: Some(100_000), cursor: None },
        )
        .map_err(|e| format!("{e:?}"))?;
        let next_id = format!("msg-{:06}", existing.records.len());
        let record = RecordWriteValue { id: next_id, payload: encode_record(&topic, &payload)? };
        store::put(RECEIVED_MESSAGES_COLLECTION, &record).map_err(|e| format!("{e:?}"))
    }
}

impl TestDriverGuest for MessagingPubsubTestComponent {
    fn subscribe_to(topic: String) -> Result<(), String> {
        host_api::subscribe(&topic).map_err(|e| format!("{e:?}"))
    }

    fn publish_to(topic: String, payload: String) -> Result<(), String> {
        host_api::publish(&topic, payload.as_bytes()).map_err(|e| format!("{e:?}"))
    }

    fn get_received_messages() -> Result<String, String> {
        let result = store::query(
            RECEIVED_MESSAGES_COLLECTION,
            &QueryOptions { filter: None, limit: Some(100_000), cursor: None },
        )
        .map_err(|e| format!("{e:?}"))?;
        let mut records = result.records;
        records.sort_by(|a, b| a.id.cmp(&b.id));
        let mut lines = Vec::with_capacity(records.len());
        for r in &records {
            let (topic, payload) = decode_record(&r.payload)?;
            let payload_str =
                String::from_utf8(payload).map_err(|e| format!("payload not UTF-8: {e}"))?;
            lines.push(format!("{topic}\t{payload_str}"));
        }
        Ok(lines.join("\n"))
    }
}
