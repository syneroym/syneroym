//! JSON request/response shapes for `blob-store`'s streaming methods over
//! native (non-WASM) JSON-RPC dispatch.
//!
//! The one-shot methods (`put-blob`/`get-blob`/`delete-blob`/`signed-url`)
//! and the data-layer/vault/app-config interfaces all reuse the existing
//! `wasmtime::component::bindgen!`-generated types directly for their
//! native JSON shape (see `crates/control_plane/src/synsvc_native.rs`).
//! Streaming has no such generated equivalent to reuse, by construction:
//! WIT `resource` handles (`blob-writer`/`blob-reader`) aren't
//! JSON-representable, so native callers get an explicit opaque session id
//! instead of a resource handle. These types are colocated here (the crate
//! that owns the streaming concept) rather than in `control_plane`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct OpenUploadResponse {
    pub upload_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WriteChunkRequest {
    pub upload_id: String,
    pub chunk: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionIdRequest {
    pub upload_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FinishUploadResponse {
    pub hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OpenDownloadRequest {
    pub hash: String,
    #[serde(default)]
    pub offset: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OpenDownloadResponse {
    pub download_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReadChunkRequest {
    pub download_id: String,
    pub max_bytes: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReadChunkResponse {
    pub chunk: Vec<u8>,
    pub eof: bool,
}
