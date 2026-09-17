use super::*;

impl ObjectStoreUploadSession {
    /// Refunds whatever this session still has speculatively reserved
    /// against the aggregate quota (a no-op if there's nothing left), and
    /// zeroes `reserved` so a second call -- from `Drop`, after `finish`/
    /// `abort` already settled it one way or the other -- can't refund
    /// twice.
    fn refund_reserved(&mut self) {
        if self.reserved > 0
            && let Ok(mut usage) = self.usage.lock()
        {
            let entry = usage.entry(self.service_id.clone()).or_insert(0);
            *entry = entry.saturating_sub(self.reserved);
        }
        self.reserved = 0;
    }
}

/// Covers every path that ends a session without going through `finish`'s
/// commit -- an explicit `abort`, or a caller simply dropping the writer
/// (the WIT resource destructor on the WASM build; `NativeBlobWriter`'s own
/// drop natively) -- with one synchronous, deterministic refund, run
/// exactly once regardless of which of those paths got here. Safe to make
/// synchronous: `usage` is a plain `std::sync::Mutex`, no I/O involved.
impl Drop for ObjectStoreUploadSession {
    fn drop(&mut self) {
        self.refund_reserved();
    }
}

#[async_trait]
impl UploadSession for ObjectStoreUploadSession {
    async fn write(&mut self, chunk: Vec<u8>) -> Result<(), BlobError> {
        let chunk_len = chunk.len() as u64;
        let new_len = self.plaintext_len + chunk_len;
        if new_len > self.max_blob_bytes {
            return Err(BlobError::QuotaExceeded);
        }

        if let Some(max_total) = self.max_service_total_bytes {
            // Check-and-reserve under a single lock acquisition against the
            // live shared counter (not a snapshot taken at `open_upload`),
            // so concurrent uploads for the same service can't each
            // independently max out the budget. Trade-off: since the
            // content hash (and thus whether this upload is really a
            // content-addressed dedup of existing bytes) is only known at
            // `finish`, a duplicate upload started right at the quota
            // boundary can spuriously fail here even though it wouldn't
            // have grown real usage -- `finish` refunds dedup'd bytes after
            // the fact, but can't retroactively un-reject an in-flight
            // write that already hit the boundary.
            let mut usage =
                self.usage.lock().map_err(|_| BlobError::Internal("usage lock poisoned".into()))?;
            let current = *usage.get(&self.service_id).unwrap_or(&0);
            if current + chunk_len > max_total {
                return Err(BlobError::QuotaExceeded);
            }
            *usage.entry(self.service_id.clone()).or_insert(0) += chunk_len;
            self.reserved += chunk_len;
        }

        self.plaintext_hasher.update(&chunk);
        self.plaintext_len = new_len;

        match &mut self.encryptor {
            Some(enc) => self.ciphertext_buf.extend(enc.update(&chunk)?),
            None => self.ciphertext_buf.extend(chunk),
        }
        Ok(())
    }

    async fn finish(mut self: Box<Self>) -> Result<String, BlobError> {
        if let Some(enc) = self.encryptor.take() {
            self.ciphertext_buf.extend(enc.finish()?);
        }
        // `mem::replace`/`mem::take`, not a field move: `Self` now
        // implements `Drop` (for the refund-on-abandon path below), and
        // Rust forbids partially moving a field out of a type that does.
        let hash = hex::encode(mem::replace(&mut self.plaintext_hasher, Sha256::new()).finalize());
        let path = object_path(&self.service_id, &hash);

        // Content-addressed storage: if a blob with this hash already
        // exists, `put` below silently overwrites identical bytes with no
        // real growth in disk usage, so this session's `write`-time
        // reservation must be refunded rather than double-counted.
        let already_existed = self.store.head(&path).await.is_ok();

        self.store
            .put(&path, PutPayload::from(mem::take(&mut self.ciphertext_buf)))
            .await
            .map_err(BlobError::from)?;

        if already_existed {
            // Dedup: nothing new actually grew on disk, so refund what
            // `write` speculatively reserved.
            self.refund_reserved();
        } else {
            // Real growth: the reservation becomes genuine usage. Settle it
            // without refunding -- otherwise `Drop`, below, would refund
            // bytes that are now truly on disk.
            self.reserved = 0;
        }
        Ok(hash)
    }

    async fn abort(mut self: Box<Self>) {
        // The refund itself happens in `Drop`, below, when `self` goes out
        // of scope at the end of this call -- identical to what happens if
        // a caller drops the writer without calling `abort` at all, so
        // there is exactly one place that does this accounting rather than
        // two copies that could drift.
        self.ciphertext_buf.clear();
    }
}
