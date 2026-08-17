#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! WASM component implementation for demo miniapp fixture.
//!
//! Provides static UI asset serving, REST endpoints via guest HTTP handler,
//! file streaming upload/download via stream routing, and live bidirectional
//! updates over both WebSocket and Server-Sent Events.

use std::{
    cell::RefCell,
    collections::VecDeque,
    time::{SystemTime, UNIX_EPOCH},
};

use bindings::{
    Guest,
    exports::syneroym::{
        http::{
            incoming_handler::{Guest as IncomingHandlerGuest, HttpRequest, HttpResponse},
            websocket_handler::Guest as WebsocketHandlerGuest,
        },
        messaging::{
            guest_api::Guest as GuestApiGuest,
            stream_types::{
                Guest as StreamTypesGuest, GuestStreamCursor, GuestStreamSink, StreamCursor,
                StreamSink,
            },
        },
    },
    syneroym::{
        blob_store::blob_store,
        data_layer::store,
        http::{websocket, websocket_types::FrameKind},
        messaging::host_api,
    },
};

mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "miniapp-demo1-wasm",
        with: {
            "syneroym:data-layer/store@0.1.0": generate,
            "syneroym:blob-store/blob-store@0.1.0": generate,
            "syneroym:messaging/host-api@0.1.0": generate,
            "syneroym:messaging/stream-types@0.1.0": generate,
            "syneroym:http/websocket@0.1.0": generate,
            "syneroym:http/websocket-types@0.1.0": generate,
            "syneroym:http/incoming-handler@0.1.0": generate,
            "syneroym:http/websocket-handler@0.1.0": generate,
            "syneroym:messaging/guest-api@0.1.0": generate,
        },
    });

    use super::MiniappDemo1WasmComponent;
    export!(MiniappDemo1WasmComponent);
}

const COMMENTS_COLLECTION: &str = "comments";
const FILES_COLLECTION: &str = "files";
const COMMENT_UPDATES_TOPIC: &str = "comment-updates";
const FILE_UPLOAD_PROTOCOL: &str = "file-upload";
const FILE_DOWNLOAD_PROTOCOL: &str = "file-download";
const CHUNK_SIZE: usize = 8192;

const COMMENTS_SPA_HTML: &str = include_str!("../static/dist/index.html");

pub struct FileDownloadCursor {
    chunks: RefCell<VecDeque<Vec<u8>>>,
}

impl GuestStreamCursor for FileDownloadCursor {
    fn next_chunk(&self) -> Result<Option<Vec<u8>>, String> {
        Ok(self.chunks.borrow_mut().pop_front())
    }
}

pub struct FileUploadSink {
    filename: String,
    buffer: RefCell<Vec<u8>>,
}

impl GuestStreamSink for FileUploadSink {
    fn push_chunk(&self, data: Vec<u8>) -> Result<(), String> {
        self.buffer.borrow_mut().extend_from_slice(&data);
        Ok(())
    }

    fn finalize(&self) -> Result<(), String> {
        let data = self.buffer.borrow().clone();
        let size = data.len() as u64;
        let hash = blob_store::put_blob(&data).map_err(|e| format!("{e:?}"))?;
        let file_obj = serde_json::json!({
            "name": self.filename,
            "hash": hash,
            "size": size,
        });
        let payload = serde_json::to_vec(&file_obj).map_err(|e| e.to_string())?;
        store::put(
            FILES_COLLECTION,
            &store::RecordWriteValue {
                id: self.filename.clone(),
                payload,
            },
        )
        .map_err(|e| format!("{e:?}"))?;
        Ok(())
    }
}

struct MiniappDemo1WasmComponent;

impl Guest for MiniappDemo1WasmComponent {
    fn init() -> Result<(), String> {
        store::create_collection(&store::CollectionSchema {
            name: COMMENTS_COLLECTION.to_string(),
            indexes: vec![],
        })
        .map_err(|e| format!("{e:?}"))?;

        store::create_collection(&store::CollectionSchema {
            name: FILES_COLLECTION.to_string(),
            indexes: vec![],
        })
        .map_err(|e| format!("{e:?}"))?;

        host_api::register_stream_protocol(FILE_UPLOAD_PROTOCOL)
            .map_err(|e| format!("{e:?}"))?;
        host_api::register_stream_protocol(FILE_DOWNLOAD_PROTOCOL)
            .map_err(|e| format!("{e:?}"))?;
        Ok(())
    }

    fn migrate() -> Result<(), String> {
        host_api::register_stream_protocol(FILE_UPLOAD_PROTOCOL)
            .map_err(|e| format!("{e:?}"))?;
        host_api::register_stream_protocol(FILE_DOWNLOAD_PROTOCOL)
            .map_err(|e| format!("{e:?}"))?;
        Ok(())
    }
}

impl StreamTypesGuest for MiniappDemo1WasmComponent {
    type StreamCursor = FileDownloadCursor;
    type StreamSink = FileUploadSink;
}

impl IncomingHandlerGuest for MiniappDemo1WasmComponent {
    fn handle_request(request: HttpRequest) -> Result<HttpResponse, String> {
        match (request.method.as_str(), request.route.as_str()) {
            ("GET", "/comments") => Ok(HttpResponse {
                status: 200,
                headers: vec![("content-type".to_string(), "text/html; charset=utf-8".to_string())],
                body: COMMENTS_SPA_HTML.as_bytes().to_vec(),
            }),
            ("GET", "/api/comments") => {
                let query_res = store::query(
                    COMMENTS_COLLECTION,
                    &store::QueryOptions {
                        filter: None,
                        limit: Some(100),
                        cursor: None,
                    },
                )
                .map_err(|e| format!("{e:?}"))?;

                let mut comments: Vec<serde_json::Value> = Vec::new();
                for rec in query_res.records {
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&rec.payload) {
                        comments.push(v);
                    }
                }
                comments.reverse();
                comments.truncate(5);

                let json_bytes = serde_json::to_vec(&comments).map_err(|e| e.to_string())?;
                Ok(HttpResponse {
                    status: 200,
                    headers: vec![("content-type".to_string(), "application/json".to_string())],
                    body: json_bytes,
                })
            }
            ("POST", "/api/comments") => {
                #[derive(serde::Deserialize)]
                struct CreateComment {
                    #[serde(default)]
                    text: String,
                }

                let payload: CreateComment = match serde_json::from_slice(&request.body) {
                    Ok(p) => p,
                    Err(e) => {
                        return Ok(HttpResponse {
                            status: 422,
                            headers: vec![("content-type".to_string(), "application/json".to_string())],
                            body: format!(r#"{{"error":"invalid json payload: {e}"}}"#).into_bytes(),
                        });
                    }
                };

                if payload.text.trim().is_empty() {
                    return Ok(HttpResponse {
                        status: 422,
                        headers: vec![("content-type".to_string(), "application/json".to_string())],
                        body: br#"{"error":"invalid payload: text is required"}"#.to_vec(),
                    });
                }

                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;

                let comment_obj = serde_json::json!({
                    "id": now_ms,
                    "text": payload.text,
                });
                let comment_bytes = serde_json::to_vec(&comment_obj).map_err(|e| e.to_string())?;

                store::put(
                    COMMENTS_COLLECTION,
                    &store::RecordWriteValue {
                        id: now_ms.to_string(),
                        payload: comment_bytes,
                    },
                )
                .map_err(|e| format!("{e:?}"))?;

                let update_msg = serde_json::json!({
                    "commentUpdateTimestamp": chrono::Utc::now().to_rfc3339(),
                });
                let update_bytes = serde_json::to_vec(&update_msg).unwrap_or_default();
                let _ = host_api::publish(COMMENT_UPDATES_TOPIC, &update_bytes);

                Ok(HttpResponse {
                    status: 201,
                    headers: vec![("content-type".to_string(), "application/json".to_string())],
                    body: b"{\"status\":\"created\"}".to_vec(),
                })
            }
            ("GET", "/api/files") => {
                let query_res = store::query(
                    FILES_COLLECTION,
                    &store::QueryOptions {
                        filter: None,
                        limit: Some(100),
                        cursor: None,
                    },
                )
                .map_err(|e| format!("{e:?}"))?;

                let mut files: Vec<serde_json::Value> = Vec::new();
                for rec in query_res.records {
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&rec.payload) {
                        files.push(v);
                    }
                }

                let json_bytes = serde_json::to_vec(&files).map_err(|e| e.to_string())?;
                Ok(HttpResponse {
                    status: 200,
                    headers: vec![("content-type".to_string(), "application/json".to_string())],
                    body: json_bytes,
                })
            }
            _ => Ok(HttpResponse {
                status: 404,
                headers: vec![("content-type".to_string(), "text/plain".to_string())],
                body: b"not found".to_vec(),
            }),
        }
    }
}

impl WebsocketHandlerGuest for MiniappDemo1WasmComponent {
    fn on_open(_conn: String) {}

    fn on_message(conn: String, frame: Vec<u8>, _kind: FrameKind) {
        let text = String::from_utf8_lossy(&frame);
        let ws_msg = serde_json::json!({
            "commentUpdateTimestamp": serde_json::Value::Null,
            "recdMsg": format!("Received: {text}"),
        });
        let _ = websocket::send(
            &conn,
            &serde_json::to_vec(&ws_msg).unwrap_or_default(),
            FrameKind::Text,
        );
    }

    fn on_close(_conn: String) {}
}

impl GuestApiGuest for MiniappDemo1WasmComponent {
    fn handle_message(_topic: String, _payload: Vec<u8>) -> Result<(), String> {
        Ok(())
    }

    fn handle_stream_request(
        protocol: String,
        _peer_id: String,
        request_data: Vec<u8>,
    ) -> Result<StreamCursor, String> {
        if protocol != FILE_DOWNLOAD_PROTOCOL {
            return Err(format!("unknown stream protocol: {protocol}"));
        }
        let raw_filename = String::from_utf8_lossy(&request_data).to_string();
        let sanitized = raw_filename
            .replace('\\', "/")
            .split('/')
            .last()
            .unwrap_or_default()
            .trim_matches('.')
            .to_string();
        let filename = if sanitized.is_empty() {
            raw_filename
        } else {
            sanitized
        };

        let record = store::get(FILES_COLLECTION, &filename)
            .map_err(|e| format!("{e:?}"))?
            .ok_or_else(|| "file not found".to_string())?;

        let file_obj: serde_json::Value =
            serde_json::from_slice(&record.payload).map_err(|e| e.to_string())?;
        let hash = file_obj
            .get("hash")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing hash in file record".to_string())?;

        let data = blob_store::get_blob(hash).map_err(|e| format!("{e:?}"))?;
        let chunks: VecDeque<Vec<u8>> = data.chunks(CHUNK_SIZE).map(<[u8]>::to_vec).collect();

        Ok(StreamCursor::new(FileDownloadCursor {
            chunks: RefCell::new(chunks),
        }))
    }

    fn accept_stream_upload(
        protocol: String,
        _peer_id: String,
        metadata: String,
    ) -> Result<StreamSink, String> {
        if protocol != FILE_UPLOAD_PROTOCOL {
            return Err(format!("unknown stream protocol: {protocol}"));
        }
        let sanitized = metadata
            .replace('\\', "/")
            .split('/')
            .last()
            .unwrap_or_default()
            .trim_matches('.')
            .to_string();
        let filename = if sanitized.is_empty() {
            "uploaded.dat".to_string()
        } else {
            sanitized
        };

        Ok(StreamSink::new(FileUploadSink {
            filename,
            buffer: RefCell::new(Vec::new()),
        }))
    }
}
