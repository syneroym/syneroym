#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Bidirectional streaming test guest component (M3B Slice 6B).
//!
//! Exercises `syneroym:messaging`'s guest-implemented `stream-cursor`
//! (guest-as-source, `handle-stream-request`) and `stream-sink`
//! (guest-as-sink, `accept-stream-upload`) resources end to end.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
};

use bindings::{
    Guest,
    exports::syneroym::messaging::{
        guest_api::Guest as GuestApiGuest,
        stream_types::{
            Guest as StreamTypesGuest, GuestStreamCursor, GuestStreamSink, StreamCursor,
            StreamSink,
        },
    },
    exports::syneroym_test::stream_test::test_driver::Guest as TestDriverGuest,
    syneroym::{data_layer::store, messaging::host_api},
};

mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "stream-test",
        with: {
            "syneroym:messaging/host-api@0.1.0": generate,
            "syneroym:messaging/stream-types@0.1.0": generate,
            "syneroym:data-layer/store@0.1.0": generate,
        },
    });

    use super::StreamTestComponent;
    export!(StreamTestComponent);
}

const PROTOCOL: &str = "file-transfer";
const UPLOADS_COLLECTION: &str = "uploads";
const LATEST_UPLOAD_ID: &str = "latest";
/// Special `request-data`/`metadata` sentinels a test can pass to force this
/// fixture's guest-side decline/abort paths, since it otherwise always
/// accepts -- see `handle_stream_request`/`accept_stream_upload`.
const REJECT_SENTINEL: &str = "reject";
/// As upload `metadata`, forces `UploadSink::push-chunk` to fail after the
/// first chunk; as download `request-data`, forces
/// `FixedContentCursor::next-chunk` to fail after the first chunk.
const FAIL_AFTER_FIRST_CHUNK_SENTINEL: &str = "fail-after-first-chunk";
/// Small on purpose: forces a multi-chunk transfer over a handful of
/// `next-chunk`/`push-chunk` calls instead of one, so integration tests
/// exercise the actual pull/push loop rather than a single round trip.
const CHUNK_SIZE: usize = 8;

fn chunk_bytes(data: &[u8], chunk_size: usize) -> VecDeque<Vec<u8>> {
    data.chunks(chunk_size.max(1)).map(<[u8]>::to_vec).collect()
}

/// Deterministic download payload derived from the request, so a test can
/// independently compute the expected bytes without the fixture needing any
/// prior state.
fn build_download_payload(peer_id: &str, request_data: &[u8]) -> Vec<u8> {
    format!("stream-test:{peer_id}:{}", String::from_utf8_lossy(request_data)).into_bytes()
}

pub struct FixedContentCursor {
    remaining: RefCell<VecDeque<Vec<u8>>>,
    /// When `true`, `next-chunk` returns `Err` once at least one chunk has
    /// already been served successfully, simulating a mid-download guest
    /// failure (mirrors `UploadSink::fail_after_first_chunk`; see
    /// `FAIL_AFTER_FIRST_CHUNK_SENTINEL`).
    fail_after_first_chunk: bool,
    chunks_served: Cell<u32>,
}

impl GuestStreamCursor for FixedContentCursor {
    fn next_chunk(&self) -> Result<Option<Vec<u8>>, String> {
        let n = self.chunks_served.get();
        self.chunks_served.set(n + 1);
        if self.fail_after_first_chunk && n >= 1 {
            return Err("simulated next-chunk failure".to_string());
        }
        Ok(self.remaining.borrow_mut().pop_front())
    }
}

pub struct UploadSink {
    buffer: RefCell<Vec<u8>>,
    /// When `true`, every `push-chunk` call after the first returns `Err`,
    /// simulating a mid-upload guest failure (see
    /// `FAIL_AFTER_FIRST_CHUNK_SENTINEL`).
    fail_after_first_chunk: bool,
    chunks_pushed: Cell<u32>,
}

impl GuestStreamSink for UploadSink {
    fn push_chunk(&self, data: Vec<u8>) -> Result<(), String> {
        let n = self.chunks_pushed.get();
        self.chunks_pushed.set(n + 1);
        if self.fail_after_first_chunk && n >= 1 {
            return Err("simulated push-chunk failure".to_string());
        }
        self.buffer.borrow_mut().extend_from_slice(&data);
        Ok(())
    }

    fn finalize(&self) -> Result<(), String> {
        // `data-layer/store::put` validates the payload is JSON at the host
        // boundary, so the raw uploaded bytes are wrapped as a JSON string
        // rather than stored verbatim (mirrors `messaging-pubsub-test`'s own
        // fixture, which hit the same requirement in Slice 6A).
        let content = self.buffer.borrow().clone();
        let payload_str = String::from_utf8_lossy(&content).into_owned();
        let json_payload =
            serde_json::to_vec(&payload_str).map_err(|e| format!("failed to encode: {e}"))?;
        let record =
            store::RecordWriteValue { id: LATEST_UPLOAD_ID.to_string(), payload: json_payload };
        store::put(UPLOADS_COLLECTION, &record).map_err(|e| format!("{e:?}"))
    }
}

struct StreamTestComponent;

impl Guest for StreamTestComponent {
    fn init() -> Result<(), String> {
        store::create_collection(&store::CollectionSchema {
            name: UPLOADS_COLLECTION.to_string(),
            indexes: vec![],
        })
        .map_err(|e| format!("{e:?}"))?;
        host_api::register_stream_protocol(PROTOCOL).map_err(|e| format!("{e:?}"))
    }

    fn migrate() -> Result<(), String> {
        // Idempotent: re-registering the same protocol on every re-deploy
        // is a last-write-wins upsert (ADR-0014), not an error.
        host_api::register_stream_protocol(PROTOCOL).map_err(|e| format!("{e:?}"))
    }
}

impl StreamTypesGuest for StreamTestComponent {
    type StreamCursor = FixedContentCursor;
    type StreamSink = UploadSink;
}

impl GuestApiGuest for StreamTestComponent {
    fn handle_stream_request(
        protocol: String,
        peer_id: String,
        request_data: Vec<u8>,
    ) -> Result<StreamCursor, String> {
        if protocol != PROTOCOL {
            return Err(format!("unknown stream protocol: {protocol}"));
        }
        if request_data == REJECT_SENTINEL.as_bytes() {
            return Err("download request rejected (test sentinel)".to_string());
        }
        let payload = build_download_payload(&peer_id, &request_data);
        let cursor = FixedContentCursor {
            remaining: RefCell::new(chunk_bytes(&payload, CHUNK_SIZE)),
            fail_after_first_chunk: request_data == FAIL_AFTER_FIRST_CHUNK_SENTINEL.as_bytes(),
            chunks_served: Cell::new(0),
        };
        Ok(StreamCursor::new(cursor))
    }

    fn accept_stream_upload(
        protocol: String,
        _peer_id: String,
        metadata: String,
    ) -> Result<StreamSink, String> {
        if protocol != PROTOCOL {
            return Err(format!("unknown stream protocol: {protocol}"));
        }
        if metadata == REJECT_SENTINEL {
            return Err("upload rejected (test sentinel)".to_string());
        }
        let sink = UploadSink {
            buffer: RefCell::new(Vec::new()),
            fail_after_first_chunk: metadata == FAIL_AFTER_FIRST_CHUNK_SENTINEL,
            chunks_pushed: Cell::new(0),
        };
        Ok(StreamSink::new(sink))
    }

    fn handle_message(_topic: String, _payload: Vec<u8>) -> Result<(), String> {
        // This fixture doesn't exercise pub/sub (Slice 6A); declared only
        // because `guest-api` requires it.
        Err("handle-message not supported by this fixture".to_string())
    }
}

impl TestDriverGuest for StreamTestComponent {
    fn register_protocol() -> Result<(), String> {
        host_api::register_stream_protocol(PROTOCOL).map_err(|e| format!("{e:?}"))
    }

    fn get_uploaded_content() -> Result<String, String> {
        match store::get(UPLOADS_COLLECTION, LATEST_UPLOAD_ID).map_err(|e| format!("{e:?}"))? {
            Some(record) => serde_json::from_slice(&record.payload)
                .map_err(|e| format!("uploaded content not valid JSON string: {e}")),
            None => Ok(String::new()),
        }
    }
}
