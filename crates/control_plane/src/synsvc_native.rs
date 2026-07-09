//! Native (non-WASM) JSON-RPC dispatch for a deployed `SynSvc`'s
//! data-layer/vault/app-config/blob-store capabilities.
//!
//! One instance is registered per deployed `service_id` in
//! `ControlPlaneService::deploy` (`crates/control_plane/src/service.rs`),
//! mirroring the same host-provided capabilities the WASM `Host` trait
//! impls in `crates/sandbox_app/src/engine.rs` expose to guests -- this is
//! the second, independent adapter over the same underlying
//! `StorageProvider`/`ServiceStore`/`BlobProvider` traits, not a
//! reimplementation of their logic. Does **not** depend on
//! `syneroym-sandbox-app`: that crate is an optional, feature-gated
//! dependency of `control_plane` (see `crate::dummy_sandbox`), and native
//! data-layer/blob-store access must work even in builds without the WASM
//! sandbox feature enabled.

use std::{collections::HashMap, sync::Arc};

use syneroym_data_blob::{
    BlobError, BlobProvider,
    native_types::{
        FinishUploadResponse, OpenDownloadRequest, OpenDownloadResponse, OpenUploadResponse,
        ReadChunkRequest, ReadChunkResponse, SessionIdRequest, WriteChunkRequest,
    },
    traits::{DownloadSession, UploadSession},
};
use syneroym_data_db::traits::{ServiceStore, StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_rpc::{NativeInvocation, NativeResponse, NativeService, RpcError, RpcResult};
use syneroym_wit_interfaces::host::syneroym::{
    app_config::app_config::ConfigError,
    data_layer::store::{
        CollectionSchema, DataLayerError, IndexDefinition, IndexType, Mutation, PatchMutation,
        QueryOptions, RecordWriteValue,
    },
    vault::vault::VaultError,
};
use tokio::sync::Mutex;
use uuid::Uuid;
use zeroize::Zeroizing;

pub struct SynSvcNativeService {
    service_id: String,
    key_store: Arc<KeyStore>,
    storage_provider: Arc<dyn StorageProvider>,
    blob_provider: Arc<dyn BlobProvider>,
    upload_sessions: Mutex<HashMap<String, Box<dyn UploadSession>>>,
    download_sessions: Mutex<HashMap<String, Box<dyn DownloadSession>>>,
}

impl std::fmt::Debug for SynSvcNativeService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SynSvcNativeService")
            .field("service_id", &self.service_id)
            .finish_non_exhaustive()
    }
}

fn internal(msg: impl std::fmt::Display) -> RpcError {
    RpcError::InternalError(msg.to_string())
}

fn invalid_params(msg: impl std::fmt::Display) -> RpcError {
    RpcError::InvalidParams(msg.to_string())
}

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

fn parse_params<T: serde::de::DeserializeOwned>(invocation: &NativeInvocation) -> RpcResult<T> {
    serde_json::from_value(invocation.params.clone())
        .map_err(|e| invalid_params(format!("invalid params for {}: {e}", invocation.method)))
}

fn to_payload<T: serde::Serialize>(value: &T) -> RpcResult<NativeResponse> {
    serde_json::to_value(value)
        .map(|payload| NativeResponse { payload })
        .map_err(|e| internal(format!("failed to serialize response: {e}")))
}

impl SynSvcNativeService {
    #[must_use]
    pub fn new(
        service_id: String,
        key_store: Arc<KeyStore>,
        storage_provider: Arc<dyn StorageProvider>,
        blob_provider: Arc<dyn BlobProvider>,
    ) -> Self {
        Self {
            service_id,
            key_store,
            storage_provider,
            blob_provider,
            upload_sessions: Mutex::new(HashMap::new()),
            download_sessions: Mutex::new(HashMap::new()),
        }
    }

    async fn open_store(&self) -> Result<Box<dyn ServiceStore>, DataLayerError> {
        self.storage_provider
            .open_service_db(&self.service_id, &self.key_store)
            .await
            .map_err(|e| DataLayerError::Internal(e.to_string()))
    }

    async fn resolve_blob_dek(&self) -> RpcResult<Option<Zeroizing<[u8; 32]>>> {
        self.storage_provider
            .load_service_dek(&self.service_id, &self.key_store)
            .await
            .map_err(internal)
    }

    // -- data-layer -----------------------------------------------------

    async fn dispatch_data_layer(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        let store = self.open_store().await.map_err(|e| internal(e.to_string()))?;
        match invocation.method.as_str() {
            "create-collection" | "create_collection" => {
                // Hand-rolled DTO: the bindgen-generated `IndexDefinition`
                // escapes the WIT `type` field as `type_` (a reserved
                // keyword), which doesn't match the plain `type` a JSON
                // caller would naturally send.
                #[derive(serde::Deserialize)]
                struct IndexDefinitionDto {
                    field_name: String,
                    #[serde(rename = "type")]
                    index_type: IndexType,
                }
                #[derive(serde::Deserialize)]
                struct Req {
                    name: String,
                    #[serde(default)]
                    indexes: Vec<IndexDefinitionDto>,
                }
                let req: Req = parse_params(&invocation)?;
                let schema = CollectionSchema {
                    name: req.name,
                    indexes: req
                        .indexes
                        .into_iter()
                        .map(|i| IndexDefinition { field_name: i.field_name, type_: i.index_type })
                        .collect(),
                };
                store.create_collection(&schema).await.map_err(|e| internal(e.to_string()))?;
                to_payload(&())
            }
            "drop-collection" | "drop_collection" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    name: String,
                }
                let req: Req = parse_params(&invocation)?;
                store.drop_collection(&req.name).await.map_err(|e| internal(e.to_string()))?;
                to_payload(&())
            }
            "put" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    value: RecordWriteValue,
                }
                let req: Req = parse_params(&invocation)?;
                store
                    .put(&req.collection, &req.value, &self.service_id)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                to_payload(&())
            }
            "patch" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    id: String,
                    patch_json: Vec<u8>,
                }
                let req: Req = parse_params(&invocation)?;
                store
                    .patch(&req.collection, &req.id, &req.patch_json)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                to_payload(&())
            }
            "get" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    id: String,
                }
                let req: Req = parse_params(&invocation)?;
                let result = store
                    .get(&req.collection, &req.id)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                to_payload(&result)
            }
            "query" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    opts: QueryOptions,
                }
                let req: Req = parse_params(&invocation)?;
                let result = store
                    .query(&req.collection, &req.opts)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                to_payload(&result)
            }
            "delete" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    id: String,
                }
                let req: Req = parse_params(&invocation)?;
                store
                    .delete(&req.collection, &req.id)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                to_payload(&())
            }
            "delete-many" | "delete_many" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    filter: Option<String>,
                }
                let req: Req = parse_params(&invocation)?;
                let affected = store
                    .delete_many(&req.collection, req.filter.as_deref())
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                to_payload(&affected)
            }
            "batch-mutate" | "batch_mutate" => {
                // Hand-rolled DTO: the bindgen-generated `Mutation` variant
                // derives serde's default externally-tagged representation
                // (e.g. `{"Put": {...}}`, PascalCase), which doesn't match
                // this API's snake_case JSON convention.
                #[derive(serde::Deserialize)]
                #[serde(tag = "type", content = "value", rename_all = "snake_case")]
                enum MutationDto {
                    Put(RecordWriteValue),
                    Patch(PatchMutation),
                    Delete(String),
                }
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    mutations: Vec<MutationDto>,
                }
                let req: Req = parse_params(&invocation)?;
                let mutations: Vec<Mutation> = req
                    .mutations
                    .into_iter()
                    .map(|m| match m {
                        MutationDto::Put(v) => Mutation::Put(v),
                        MutationDto::Patch(v) => Mutation::Patch(v),
                        MutationDto::Delete(v) => Mutation::Delete(v),
                    })
                    .collect();
                store
                    .batch_mutate(&req.collection, &mutations, &self.service_id)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                to_payload(&())
            }
            "execute-ddl" | "execute_ddl" => {
                // Native callers are never in a WASM init()/migrate()
                // lifecycle context, so this is always denied -- matches
                // the WASM host's own `is_init_context` gate.
                // TODO(M4): replace with an Admin UCAN capability check,
                // same as the WASM path's TODO in engine.rs.
                Err(internal(DataLayerError::PermissionDenied.to_string()))
            }
            other => Err(RpcError::MethodNotFound(format!("data-layer/{other}"))),
        }
    }

    // -- vault ------------------------------------------------------------

    async fn dispatch_vault(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        match invocation.method.as_str() {
            "reveal" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    key: String,
                }
                let req: Req = parse_params(&invocation)?;
                let store = self
                    .storage_provider
                    .open_service_db(&self.service_id, &self.key_store)
                    .await
                    .map_err(internal)?;
                match store.reveal_secret(&req.key).await.map_err(internal)? {
                    Some(bytes) => to_payload(&bytes),
                    None => Err(internal(VaultError::NotFound.to_string())),
                }
            }
            other => Err(RpcError::MethodNotFound(format!("vault/{other}"))),
        }
    }

    // -- app-config ---------------------------------------------------------

    async fn dispatch_app_config(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        // Generation is resolved fresh per call, the native-dispatch
        // equivalent of "pinned at invocation start" (ADR-0008) -- each RPC
        // call *is* its own invocation here, there's no longer-lived Store
        // to pin a generation on ahead of time the way a WASM guest's does.
        let generation = self
            .storage_provider
            .get_latest_config_generation(&self.service_id)
            .await
            .map_err(internal)?;

        match invocation.method.as_str() {
            "get" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    key: String,
                }
                let req: Req = parse_params(&invocation)?;
                let Some((_, blob)) = generation else {
                    return to_payload(&Option::<String>::None);
                };
                let json: serde_json::Value = serde_json::from_str(&blob)
                    .map_err(|e| internal(ConfigError::Internal(e.to_string()).to_string()))?;
                let val = json.get(&req.key).and_then(|v| v.as_str()).map(str::to_string);
                to_payload(&val)
            }
            "get-section" | "get_section" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    prefix: String,
                }
                let req: Req = parse_params(&invocation)?;
                let Some((_, blob)) = generation else {
                    return to_payload(&Vec::<(String, String)>::new());
                };
                let json: serde_json::Value = serde_json::from_str(&blob)
                    .map_err(|e| internal(ConfigError::Internal(e.to_string()).to_string()))?;
                let mut results = Vec::new();
                if let serde_json::Value::Object(map) = json {
                    for (k, v) in map {
                        if (k == req.prefix || k.starts_with(&format!("{}.", req.prefix)))
                            && let Some(s) = v.as_str()
                        {
                            results.push((k, s.to_string()));
                        }
                    }
                }
                to_payload(&results)
            }
            other => Err(RpcError::MethodNotFound(format!("app-config/{other}"))),
        }
    }

    // -- blob-store -----------------------------------------------------

    async fn dispatch_blob_store(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        let dek = self.resolve_blob_dek().await?;
        match invocation.method.as_str() {
            "put-blob" | "put_blob" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    data: Vec<u8>,
                }
                let req: Req = parse_params(&invocation)?;
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
                let url = self
                    .blob_provider
                    .signed_url(&self.service_id, &req.hash, req.ttl_secs, dek)
                    .await
                    .map_err(blob_error)?;
                to_payload(&url)
            }
            "open-upload" | "open_upload" => {
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
            other => Err(RpcError::MethodNotFound(format!("blob-store/{other}"))),
        }
    }
}

#[async_trait::async_trait]
impl NativeService for SynSvcNativeService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        match invocation.interface.as_str() {
            "data-layer" => self.dispatch_data_layer(invocation).await,
            "vault" => self.dispatch_vault(invocation).await,
            "app-config" => self.dispatch_app_config(invocation).await,
            "blob-store" => self.dispatch_blob_store(invocation).await,
            other => Err(RpcError::MethodNotFound(format!("unknown interface: {other}"))),
        }
    }
}
