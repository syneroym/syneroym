//! `syneroym_chunk_transfer::{ChunkSource, ChunkSink}` impls for this
//! crate's own session traits, so `blob-store` and `syneroym:messaging`'s
//! Slice 6B stream resources (`crates/sandbox_wasm/src/stream.rs`) share one
//! push/pull loop implementation instead of each maintaining its own.

use async_trait::async_trait;
use syneroym_chunk_transfer::{ChunkSink, ChunkSource};

use crate::traits::{DownloadSession, UploadSession};

#[async_trait]
impl ChunkSource for Box<dyn DownloadSession> {
    async fn next_chunk(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        let chunk = self.as_mut().read(u32::MAX).await?;
        if chunk.is_empty() { Ok(None) } else { Ok(Some(chunk)) }
    }
}

#[async_trait]
impl ChunkSink for Box<dyn UploadSession> {
    async fn push_chunk(&mut self, data: Vec<u8>) -> anyhow::Result<()> {
        self.as_mut().write(data).await?;
        Ok(())
    }

    async fn finalize(self: Box<Self>) -> anyhow::Result<()> {
        (*self).finish().await?;
        Ok(())
    }

    async fn abort(self: Box<Self>) {
        (*self).abort().await;
    }
}
