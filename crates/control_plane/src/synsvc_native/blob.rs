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

    /// DEK resolution is a keystore/DB round trip -- only resolved by the
    /// arms below that actually pass it to `blob_provider`.
    /// `write-chunk`/`read-chunk`/`finish-upload`/`abort-upload`/
    /// `close-download` operate on an already-open session (the DEK was
    /// already used, once, at `open-upload`/`open-download` time) and must
    /// not re-resolve it on every chunk -- a per-chunk resolve here would
    /// mean one DB/keystore query per 64KB streamed.
    pub(super) async fn dispatch_blob_store(
        &self,
        invocation: NativeInvocation,
    ) -> RpcResult<NativeResponse> {
        match invocation.method.as_str() {
            "put-blob" | "put_blob" => self.blob_put(invocation).await,
            "get-blob" | "get_blob" => self.blob_get(invocation).await,
            "delete-blob" | "delete_blob" => self.blob_delete(invocation).await,
            "signed-url" | "signed_url" => self.blob_signed_url(invocation).await,
            "open-upload" | "open_upload" => self.blob_open_upload().await,
            "write-chunk" | "write_chunk" => self.blob_write_chunk(invocation).await,
            "finish-upload" | "finish_upload" => self.blob_finish_upload(invocation).await,
            "abort-upload" | "abort_upload" => self.blob_abort_upload(invocation).await,
            "open-download" | "open_download" => self.blob_open_download(invocation).await,
            "read-chunk" | "read_chunk" => self.blob_read_chunk(invocation).await,
            "close-download" | "close_download" => self.blob_close_download(invocation).await,
            other => Err(RpcError::MethodNotFound(format!("blob-store/{other}"))),
        }
    }

    async fn blob_put(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
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

    async fn blob_get(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
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

    async fn blob_delete(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        #[derive(serde::Deserialize)]
        struct Req {
            hash: String,
        }
        let req: Req = parse_params(&invocation)?;
        self.blob_provider.delete_blob(&self.service_id, &req.hash).await.map_err(blob_error)?;
        to_payload(&())
    }

    async fn blob_signed_url(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
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

    async fn blob_open_upload(&self) -> RpcResult<NativeResponse> {
        let dek = self.resolve_blob_dek().await?;
        let session =
            self.blob_provider.open_upload(&self.service_id, dek).await.map_err(blob_error)?;
        let upload_id = Uuid::new_v4().to_string();
        self.upload_sessions.lock().await.insert(upload_id.clone(), session);
        to_payload(&OpenUploadResponse { upload_id })
    }

    async fn blob_write_chunk(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        let req: WriteChunkRequest = parse_params(&invocation)?;
        // Held only for the lookup/reinsert, not across the I/O `.await`
        // below, so concurrent uploads for other sessions aren't
        // serialized on this one.
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

    async fn blob_finish_upload(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
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

    async fn blob_abort_upload(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        let req: SessionIdRequest = parse_params(&invocation)?;
        let session = self.upload_sessions.lock().await.remove(&req.upload_id);
        if let Some(session) = session {
            session.abort().await;
        }
        to_payload(&())
    }

    async fn blob_open_download(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
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

    async fn blob_read_chunk(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        let req: ReadChunkRequest = parse_params(&invocation)?;
        // Held only for the lookup/reinsert, not across the I/O `.await`
        // below, so concurrent downloads for other sessions aren't
        // serialized on this one.
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

    /// Best-effort session release for a download that never reaches EOF
    /// (e.g. an HTTP client disconnecting mid-stream) -- see
    /// `BlobDownloadState`'s `Drop` impl in
    /// `crates/router/src/route_handler/http.rs`. Removing an unknown/
    /// already-EOF'd `download_id` is not an error.
    async fn blob_close_download(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        let req: CloseDownloadRequest = parse_params(&invocation)?;
        self.download_sessions.lock().await.remove(&req.download_id);
        to_payload(&())
    }
}
