#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Messaging pub/sub test guest component
//!
//! Exercises `syneroym:messaging/host-api` (`subscribe`/`publish`) and the
//! optional `syneroym:messaging/guest-api::handle-message` push-delivery
//! export end-to-end for M3B Slice 6A integration tests.

use bindings::{
    Guest,
    exports::{
        syneroym::messaging::{
            guest_api::Guest as GuestApiGuest,
            stream_types::{Guest as StreamTypesGuest, GuestStreamCursor, GuestStreamSink},
        },
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
            "syneroym:messaging/stream-types@0.1.0": generate,
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

/// This fixture doesn't exercise M3B Slice 6B streaming -- see
/// `test-components/stream-test` for those fixtures -- but must still
/// satisfy `guest-api`'s `use stream-types.{stream-cursor, stream-sink}`
/// reference, since `stream-types` is guest-implemented (ADR-0014). Never
/// constructed in practice: `handle-stream-request`/`accept-stream-upload`
/// below always return `Err` before any instance of these types would need
/// to exist.
pub struct UnusedStreamCursor;

impl GuestStreamCursor for UnusedStreamCursor {
    fn next_chunk(&self) -> Result<Option<Vec<u8>>, String> {
        Err("streaming not supported by this fixture".to_string())
    }
}

pub struct UnusedStreamSink;

impl GuestStreamSink for UnusedStreamSink {
    fn push_chunk(&self, _data: Vec<u8>) -> Result<(), String> {
        Err("streaming not supported by this fixture".to_string())
    }

    fn finalize(&self) -> Result<(), String> {
        Err("streaming not supported by this fixture".to_string())
    }
}

impl StreamTypesGuest for MessagingPubsubTestComponent {
    type StreamCursor = UnusedStreamCursor;
    type StreamSink = UnusedStreamSink;
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

    fn handle_stream_request(
        _protocol: String,
        _peer_id: String,
        _request_data: Vec<u8>,
    ) -> Result<bindings::exports::syneroym::messaging::stream_types::StreamCursor, String> {
        Err("streaming not supported by this fixture".to_string())
    }

    fn accept_stream_upload(
        _protocol: String,
        _peer_id: String,
        _metadata: String,
    ) -> Result<bindings::exports::syneroym::messaging::stream_types::StreamSink, String> {
        Err("streaming not supported by this fixture".to_string())
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
