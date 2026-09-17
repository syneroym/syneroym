use super::*;

impl ObjectStoreBlobProvider {
    /// Ensures `usage` has an entry for `service_id`, populating it via a
    /// one-time `list()` + sum if this is the first time this service has
    /// been touched since the provider was constructed (i.e. after a
    /// restart). No-op when aggregate quotas are disabled.
    async fn ensure_usage_loaded(&self, service_id: &str) -> Result<(), BlobError> {
        if self.max_service_total_bytes.is_none() {
            return Ok(());
        }
        {
            let usage =
                self.usage.lock().map_err(|_| BlobError::Internal("usage lock poisoned".into()))?;
            if usage.contains_key(service_id) {
                return Ok(());
            }
        }
        let prefix = ObjectPath::from(service_id.to_string());
        let mut total: u64 = 0;
        let mut stream = self.store.list(Some(&prefix));
        while let Some(meta) = stream.next().await {
            let meta = meta.map_err(BlobError::from)?;
            total += meta.size;
        }
        let mut usage =
            self.usage.lock().map_err(|_| BlobError::Internal("usage lock poisoned".into()))?;
        usage.entry(service_id.to_string()).or_insert(total);
        Ok(())
    }

    fn record_usage(&self, service_id: &str, delta: i64) {
        if let Ok(mut usage) = self.usage.lock() {
            let entry = usage.entry(service_id.to_string()).or_insert(0);
            *entry = (i64::try_from(*entry).unwrap_or(i64::MAX) + delta).max(0) as u64;
        }
    }

    /// Defense-in-depth on top of the regex validation in `validate_hash`:
    /// only meaningful for the `LocalFileSystem` backend, where `local_root`
    /// is set.
    fn check_local_path_traversal(&self, service_id: &str, hash: &str) -> Result<(), BlobError> {
        if let Some(local_root) = &self.local_root {
            let resolved = local_root.join(service_id).join(&hash[0..2]).join(&hash[2..]);
            if !resolved.starts_with(local_root) {
                return Err(BlobError::Internal("path traversal rejected".to_string()));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl BlobProvider for ObjectStoreBlobProvider {
    async fn open_upload(
        &self,
        service_id: &str,
        dek: Option<Zeroizing<[u8; 32]>>,
    ) -> Result<Box<dyn UploadSession>, BlobError> {
        validate_service_id(service_id)?;
        self.ensure_usage_loaded(service_id).await?;

        let (encryptor, header) = match dek {
            Some(dek) => {
                let (enc, header) = BlobEncryptor::new(&dek, service_id);
                (Some(enc), header)
            }
            None => (None, Vec::new()),
        };

        Ok(Box::new(ObjectStoreUploadSession {
            store: self.store.clone(),
            service_id: service_id.to_string(),
            max_blob_bytes: self.max_blob_bytes,
            max_service_total_bytes: self.max_service_total_bytes,
            reserved: 0,
            usage: self.usage.clone(),
            plaintext_hasher: Sha256::new(),
            plaintext_len: 0,
            encryptor,
            ciphertext_buf: header,
        }))
    }

    async fn open_download(
        &self,
        service_id: &str,
        hash: &str,
        offset: u64,
        dek: Option<Zeroizing<[u8; 32]>>,
    ) -> Result<Box<dyn DownloadSession>, BlobError> {
        validate_service_id(service_id)?;
        validate_hash(hash)?;
        let path = object_path(service_id, hash);
        self.check_local_path_traversal(service_id, hash)?;

        // Unencrypted blobs have no header/segment framing to walk through,
        // so a nonzero `offset` can be satisfied with a backend-side ranged
        // read instead of transferring (and discarding) everything before
        // it. Trade-off: a ranged read can only be checked against the
        // whole-content hash by hashing from byte 0, which defeats the
        // point, so integrity verification is skipped for this path
        // specifically -- `offset == 0` reads (including every `get_blob`
        // call) and all encrypted reads are unaffected.
        let (stream, verify_full_hash, skip_remaining) = if dek.is_none() && offset > 0 {
            let meta = self.store.head(&path).await.map_err(BlobError::from)?;
            let stream = if offset >= meta.size {
                futures::stream::empty::<object_store::Result<Bytes>>().boxed()
            } else {
                let opts = object_store::GetOptions {
                    range: Some(object_store::GetRange::Offset(offset)),
                    ..Default::default()
                };
                self.store.get_opts(&path, opts).await.map_err(BlobError::from)?.into_stream()
            };
            (stream, false, 0)
        } else {
            let get_result = self.store.get(&path).await.map_err(BlobError::from)?;
            (get_result.into_stream(), true, offset)
        };

        Ok(Box::new(ObjectStoreDownloadSession {
            stream,
            raw_buf: Vec::new(),
            pending_out: Vec::new(),
            decryptor: None,
            header_consumed: dek.is_none(),
            dek,
            service_id: service_id.to_string(),
            expected_hash: hash.to_string(),
            plaintext_hasher: Sha256::new(),
            offset_remaining_to_skip: skip_remaining,
            verify_full_hash,
            eof_reached: false,
            finalized: false,
        }))
    }

    async fn delete_blob(&self, service_id: &str, hash: &str) -> Result<(), BlobError> {
        validate_service_id(service_id)?;
        validate_hash(hash)?;
        let path = object_path(service_id, hash);
        self.check_local_path_traversal(service_id, hash)?;

        let prior_size = self.store.head(&path).await.ok().map(|meta| meta.size);

        match self.store.delete(&path).await {
            Ok(()) => {
                if let Some(size) = prior_size {
                    self.record_usage(service_id, -(size as i64));
                }
            }
            Err(object_store::Error::NotFound { .. }) => {}
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    async fn signed_url(
        &self,
        service_id: &str,
        hash: &str,
        ttl_secs: u32,
        dek: Option<Zeroizing<[u8; 32]>>,
    ) -> Result<String, BlobError> {
        validate_service_id(service_id)?;
        validate_hash(hash)?;
        let path = object_path(service_id, hash);
        self.check_local_path_traversal(service_id, hash)?;
        self.store.head(&path).await.map_err(BlobError::from)?;

        let dek = dek.unwrap_or_default();
        let now = time::SystemTime::now()
            .duration_since(time::UNIX_EPOCH)
            .map_err(|e| BlobError::Internal(e.to_string()))?
            .as_secs();
        Ok(sign_url(&dek, service_id, hash, ttl_secs, now))
    }
}
