use super::*;

fn map_blob_error(e: BlobStoreError) -> BlobError {
    match e {
        BlobStoreError::NotFound => BlobError::NotFound,
        BlobStoreError::QuotaExceeded => BlobError::QuotaExceeded,
        BlobStoreError::Internal(msg) => BlobError::Internal(msg),
    }
}

/// Resolves the calling component's DEK for blob encryption. `Ok(None)`
/// means `storage.encryption = false`; blobs are then stored in plaintext.
async fn resolve_blob_dek(
    component_id: &str,
    key_store: &Arc<KeyStore>,
    storage_provider: &Arc<dyn StorageProvider>,
) -> Result<Option<Zeroizing<[u8; 32]>>, BlobError> {
    storage_provider
        .load_service_dek(component_id, key_store)
        .await
        .map_err(|e| BlobError::Internal(e.to_string()))
}

impl blob_store::Host for HostState {
    async fn put_blob(&mut self, data: Vec<u8>) -> Result<String, BlobError> {
        if self.read_only {
            return Err(BlobError::Internal(
                "stage-4 after-step instances are read-only".to_string(),
            ));
        }
        let dek =
            resolve_blob_dek(&self.component_id, &self.key_store, &self.storage_provider).await?;
        self.blob_provider.put_blob(&self.component_id, data, dek).await.map_err(map_blob_error)
    }

    async fn get_blob(&mut self, hash: String) -> Result<Vec<u8>, BlobError> {
        let dek =
            resolve_blob_dek(&self.component_id, &self.key_store, &self.storage_provider).await?;
        self.blob_provider.get_blob(&self.component_id, &hash, dek).await.map_err(map_blob_error)
    }

    async fn open_upload(&mut self) -> Result<Resource<BlobWriter>, BlobError> {
        if self.read_only {
            return Err(BlobError::Internal(
                "stage-4 after-step instances are read-only".to_string(),
            ));
        }
        let dek =
            resolve_blob_dek(&self.component_id, &self.key_store, &self.storage_provider).await?;
        let session = self
            .blob_provider
            .open_upload(&self.component_id, dek)
            .await
            .map_err(map_blob_error)?;
        self.table
            .push(HostUploadSession(Some(session)))
            .map_err(|e| BlobError::Internal(e.to_string()))
    }

    async fn open_download(
        &mut self,
        hash: String,
        offset: u64,
    ) -> Result<Resource<BlobReader>, BlobError> {
        let dek =
            resolve_blob_dek(&self.component_id, &self.key_store, &self.storage_provider).await?;
        let session = self
            .blob_provider
            .open_download(&self.component_id, &hash, offset, dek)
            .await
            .map_err(map_blob_error)?;
        self.table
            .push(HostDownloadSession(session))
            .map_err(|e| BlobError::Internal(e.to_string()))
    }

    async fn delete_blob(&mut self, hash: String) -> Result<(), BlobError> {
        if self.read_only {
            return Err(BlobError::Internal(
                "stage-4 after-step instances are read-only".to_string(),
            ));
        }
        self.blob_provider.delete_blob(&self.component_id, &hash).await.map_err(map_blob_error)
    }

    async fn signed_url(&mut self, hash: String, ttl_secs: u32) -> Result<String, BlobError> {
        // Every other mutating/egress host function is hard-denied under
        // `read_only`; a signed URL is a read in
        // shape but mints a time-limited, externally redeemable URL that
        // outlives this throw-away stage-4 instance -- the same kind of
        // egress-beyond-the-call ADR-0017 §7's "local, read-only lookups
        // only" is meant to rule out.
        if self.read_only {
            return Err(BlobError::Internal(
                "stage-4 after-step instances are read-only".to_string(),
            ));
        }
        let dek =
            resolve_blob_dek(&self.component_id, &self.key_store, &self.storage_provider).await?;
        self.blob_provider
            .signed_url(&self.component_id, &hash, ttl_secs, dek)
            .await
            .map_err(map_blob_error)
    }
}

impl HostBlobWriter for HostState {
    async fn write(
        &mut self,
        self_: Resource<BlobWriter>,
        chunk: Vec<u8>,
    ) -> Result<(), BlobError> {
        if self.read_only {
            return Err(BlobError::Internal(
                "stage-4 after-step instances are read-only".to_string(),
            ));
        }
        let session = self.table.get_mut(&self_).map_err(|e| BlobError::Internal(e.to_string()))?;
        let session = session
            .0
            .as_mut()
            .ok_or_else(|| BlobError::Internal("blob writer already finished".to_string()))?;
        session.write(chunk).await.map_err(map_blob_error)
    }

    async fn finish(&mut self, self_: Resource<BlobWriter>) -> Result<String, BlobError> {
        if self.read_only {
            return Err(BlobError::Internal(
                "stage-4 after-step instances are read-only".to_string(),
            ));
        }
        // `finish` is a resource *method* (`[method]blob-writer.finish`), so
        // the canonical ABI hands the host a *borrowed* `self_` -- the table
        // entry has to stay alive until wasmtime's own resource-drop call
        // for this handle arrives (`drop`, below), or a still-live borrow
        // the guest holds could have its slot handed to an unrelated
        // resource in between. Taking the inner session out of the table
        // entry (leaving `None` behind) ends the upload without deleting
        // the entry itself.
        let entry = self.table.get_mut(&self_).map_err(|e| BlobError::Internal(e.to_string()))?;
        let session = entry
            .0
            .take()
            .ok_or_else(|| BlobError::Internal("blob writer already finished".to_string()))?;
        session.finish().await.map_err(map_blob_error)
    }

    async fn abort(&mut self, self_: Resource<BlobWriter>) {
        // See `finish`'s comment: `abort` is the same kind of method call.
        if let Ok(entry) = self.table.get_mut(&self_)
            && let Some(session) = entry.0.take()
        {
            session.abort().await;
        }
    }

    async fn drop(&mut self, rep: Resource<BlobWriter>) -> wasmtime::Result<()> {
        // This is the resource *destructor*, so `rep` is genuinely owned --
        // safe to delete the table entry outright. If the guest called
        // `finish`/`abort` already, the entry holds `None` and this only
        // frees the slot; if it dropped the resource without calling
        // either, abort the still-live session (implicit abort).
        if let Ok(entry) = self.table.delete(rep)
            && let Some(session) = entry.0
        {
            session.abort().await;
        }
        Ok(())
    }
}

impl HostBlobReader for HostState {
    async fn read(
        &mut self,
        self_: Resource<BlobReader>,
        max_bytes: u32,
    ) -> Result<Vec<u8>, BlobError> {
        let session = self.table.get_mut(&self_).map_err(|e| BlobError::Internal(e.to_string()))?;
        session.0.read(max_bytes).await.map_err(map_blob_error)
    }

    async fn drop(&mut self, rep: Resource<BlobReader>) -> wasmtime::Result<()> {
        let _ = self.table.delete(rep);
        Ok(())
    }
}
