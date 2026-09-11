use syneroym_data_blob::{
    BlobError,
    native_types::{
        CloseDownloadRequest, FinishUploadResponse, OpenDownloadRequest, OpenDownloadResponse,
        OpenUploadResponse, ReadChunkRequest, ReadChunkResponse, SessionIdRequest,
        WriteChunkRequest,
    },
};
use syneroym_rpc::{NativeInvocation, NativeResponse, RpcError, RpcResult};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::*;

/// Maps `BlobError` the way `engine.rs`'s `map_blob_error` does for the WASM
/// path, but into `RpcError::Custom` codes (there's no shared WIT
/// `blob-error` variant on this native-dispatch path to map onto), so a
/// caller can distinguish "not found"/"quota exceeded" from a generic
/// internal failure instead of every case collapsing into
/// `RpcError::InternalError`.
fn blob_error(e: BlobError) -> RpcError {
    match e {
        BlobError::NotFound => RpcError::Custom(-32001, "blob not found".to_string(), None),
        BlobError::QuotaExceeded => {
            RpcError::Custom(-32002, "blob quota exceeded".to_string(), None)
        }
        BlobError::Internal(msg) => internal(msg),
    }
}

impl SynSvcNativeService {
    pub(super) async fn resolve_blob_dek(&self) -> RpcResult<Option<Zeroizing<[u8; 32]>>> {
        self.storage_provider
            .load_service_dek(&self.service_id, &self.key_store)
            .await
            .map_err(internal)
    }

    // -- data-layer -----------------------------------------------------

    pub(super) async fn dispatch_blob_store(
        &self,
        invocation: NativeInvocation,
    ) -> RpcResult<NativeResponse> {
        // DEK resolution is a keystore/DB round trip -- only resolved for
        // the methods that actually pass it to `blob_provider` below.
        // `write-chunk`/`read-chunk`/`finish-upload`/`abort-upload`/
        // `close-download` operate on an already-open session (the DEK was
        // already used, once, at `open-upload`/`open-download` time) and
        // must not re-resolve it on every chunk -- a per-chunk resolve
        // here would mean one DB/keystore query per 64KB streamed.
        match invocation.method.as_str() {
            "put-blob" | "put_blob" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    data: Vec<u8>,
                }
                let req: Req = parse_params(&invocation)?;
                let dek = self.resolve_blob_dek().await?;
                let hash = self
                    .blob_provider
                    .put_blob(&self.service_id, req.data, dek)
                    .await
                    .map_err(blob_error)?;
                to_payload(&hash)
            }
            "get-blob" | "get_blob" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    hash: String,
                }
                let req: Req = parse_params(&invocation)?;
                let dek = self.resolve_blob_dek().await?;
                let data = self
                    .blob_provider
                    .get_blob(&self.service_id, &req.hash, dek)
                    .await
                    .map_err(blob_error)?;
                to_payload(&data)
            }
            "delete-blob" | "delete_blob" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    hash: String,
                }
                let req: Req = parse_params(&invocation)?;
                self.blob_provider
                    .delete_blob(&self.service_id, &req.hash)
                    .await
                    .map_err(blob_error)?;
                to_payload(&())
            }
            "signed-url" | "signed_url" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    hash: String,
                    ttl_secs: u32,
                }
                let req: Req = parse_params(&invocation)?;
                let dek = self.resolve_blob_dek().await?;
                let url = self
                    .blob_provider
                    .signed_url(&self.service_id, &req.hash, req.ttl_secs, dek)
                    .await
                    .map_err(blob_error)?;
                to_payload(&url)
            }
            "open-upload" | "open_upload" => {
                let dek = self.resolve_blob_dek().await?;
                let session = self
                    .blob_provider
                    .open_upload(&self.service_id, dek)
                    .await
                    .map_err(blob_error)?;
                let upload_id = Uuid::new_v4().to_string();
                self.upload_sessions.lock().await.insert(upload_id.clone(), session);
                to_payload(&OpenUploadResponse { upload_id })
            }
            "write-chunk" | "write_chunk" => {
                let req: WriteChunkRequest = parse_params(&invocation)?;
                // Held only for the lookup/reinsert, not across the I/O
                // `.await` below, so concurrent uploads for other sessions
                // aren't serialized on this one.
                let mut session = self
                    .upload_sessions
                    .lock()
                    .await
                    .remove(&req.upload_id)
                    .ok_or_else(|| invalid_params("unknown upload_id"))?;
                let result = session.write(req.chunk).await;
                self.upload_sessions.lock().await.insert(req.upload_id, session);
                result.map_err(blob_error)?;
                to_payload(&())
            }
            "finish-upload" | "finish_upload" => {
                let req: SessionIdRequest = parse_params(&invocation)?;
                let session = self
                    .upload_sessions
                    .lock()
                    .await
                    .remove(&req.upload_id)
                    .ok_or_else(|| invalid_params("unknown upload_id"))?;
                let hash = session.finish().await.map_err(blob_error)?;
                to_payload(&FinishUploadResponse { hash })
            }
            "abort-upload" | "abort_upload" => {
                let req: SessionIdRequest = parse_params(&invocation)?;
                let session = self.upload_sessions.lock().await.remove(&req.upload_id);
                if let Some(session) = session {
                    session.abort().await;
                }
                to_payload(&())
            }
            "open-download" | "open_download" => {
                let req: OpenDownloadRequest = parse_params(&invocation)?;
                let dek = self.resolve_blob_dek().await?;
                let session = self
                    .blob_provider
                    .open_download(&self.service_id, &req.hash, req.offset, dek)
                    .await
                    .map_err(blob_error)?;
                let download_id = Uuid::new_v4().to_string();
                self.download_sessions.lock().await.insert(download_id.clone(), session);
                to_payload(&OpenDownloadResponse { download_id })
            }
            "read-chunk" | "read_chunk" => {
                let req: ReadChunkRequest = parse_params(&invocation)?;
                // Held only for the lookup/reinsert, not across the I/O
                // `.await` below, so concurrent downloads for other
                // sessions aren't serialized on this one.
                let mut session = self
                    .download_sessions
                    .lock()
                    .await
                    .remove(&req.download_id)
                    .ok_or_else(|| invalid_params("unknown download_id"))?;
                let chunk = match session.read(req.max_bytes).await {
                    Ok(chunk) => chunk,
                    Err(e) => {
                        self.download_sessions.lock().await.insert(req.download_id, session);
                        return Err(blob_error(e));
                    }
                };
                let eof = chunk.is_empty();
                if !eof {
                    self.download_sessions.lock().await.insert(req.download_id, session);
                }
                to_payload(&ReadChunkResponse { chunk, eof })
            }
            "close-download" | "close_download" => {
                // Best-effort session release for a download that never
                // reaches EOF (e.g. an HTTP client disconnecting mid-
                // stream) -- see `BlobDownloadState`'s `Drop` impl in
                // `crates/router/src/route_handler/http.rs`. Removing an
                // unknown/already-EOF'd `download_id` is not an error.
                let req: CloseDownloadRequest = parse_params(&invocation)?;
                self.download_sessions.lock().await.remove(&req.download_id);
                to_payload(&())
            }
            other => Err(RpcError::MethodNotFound(format!("blob-store/{other}"))),
        }
    }
}
