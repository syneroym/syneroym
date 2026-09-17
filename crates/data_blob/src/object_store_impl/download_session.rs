use super::*;

impl ObjectStoreDownloadSession {
    /// Hashes and (offset-)filters newly decoded plaintext into
    /// `pending_out`. Hashing always covers the true plaintext from byte 0,
    /// even though bytes before `offset` are never returned to the caller.
    fn feed_plaintext(&mut self, plaintext: Vec<u8>) {
        if plaintext.is_empty() {
            return;
        }
        self.plaintext_hasher.update(&plaintext);
        if self.offset_remaining_to_skip == 0 {
            self.pending_out.extend(plaintext);
            return;
        }
        let skip = self.offset_remaining_to_skip.min(plaintext.len() as u64) as usize;
        self.offset_remaining_to_skip -= skip as u64;
        self.pending_out.extend(&plaintext[skip..]);
    }

    fn verify_hash(&self) -> Result<(), BlobError> {
        if !self.verify_full_hash {
            return Ok(());
        }
        let actual = hex::encode(self.plaintext_hasher.clone().finalize());
        if actual == self.expected_hash {
            Ok(())
        } else {
            Err(BlobError::Internal("integrity check failed".to_string()))
        }
    }
}

#[async_trait]
impl DownloadSession for ObjectStoreDownloadSession {
    async fn read(&mut self, max_bytes: u32) -> Result<Vec<u8>, BlobError> {
        while self.pending_out.len() < max_bytes as usize && !self.eof_reached {
            match self.stream.next().await {
                None => {
                    self.eof_reached = true;
                    if !self.finalized {
                        self.finalized = true;
                        if let Some(dec) = self.decryptor.take() {
                            let tail = dec.finish()?;
                            self.feed_plaintext(tail);
                        } else if !self.header_consumed {
                            // Encrypted blob whose stream ended before a
                            // full header arrived -- corrupt/truncated.
                            return Err(BlobError::Internal("integrity check failed".to_string()));
                        }
                        self.verify_hash()?;
                    }
                }
                Some(Err(e)) => return Err(BlobError::Internal(e.to_string())),
                Some(Ok(bytes)) => {
                    self.raw_buf.extend_from_slice(&bytes);
                    if self.dek.is_some() {
                        if !self.header_consumed {
                            if self.raw_buf.len() < HEADER_LEN {
                                continue;
                            }
                            let header: Vec<u8> = self.raw_buf.drain(..HEADER_LEN).collect();
                            #[allow(clippy::expect_used)]
                            let dek = self.dek.clone().expect("checked is_some above");
                            self.decryptor =
                                Some(BlobDecryptor::new(&dek, &self.service_id, &header)?);
                            self.header_consumed = true;
                        }
                        if !self.raw_buf.is_empty()
                            && let Some(dec) = &mut self.decryptor
                        {
                            let plaintext = dec.update(&self.raw_buf)?;
                            self.raw_buf.clear();
                            self.feed_plaintext(plaintext);
                        }
                    } else {
                        let plaintext = mem::take(&mut self.raw_buf);
                        self.feed_plaintext(plaintext);
                    }
                }
            }
        }

        let n = (max_bytes as usize).min(self.pending_out.len());
        Ok(self.pending_out.drain(..n).collect())
    }
}
