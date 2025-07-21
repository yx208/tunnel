use std::pin::Pin;
use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;
use serde::{Deserialize, Serialize};
use crate::core::TransferError;
use super::types::TransferId;
use super::errors::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferMode {
    Upload,
    Download,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolType {
    Tus,
    Http,
    Ftp,
    BitTorrent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferMetadata {
    pub total_size: Option<u64>,
    pub resumable: bool,
    pub content_type: Option<String>,
    pub filename: Option<String>,
    pub extra: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct TransferContext {
    pub id: TransferId,

    /// Source(URL or filepath)
    pub source: String,

    /// Target(URL or filepath)
    pub destination: String,

    pub mode: TransferMode,

    pub protocol: ProtocolType,

    pub metadata: Option<TransferMetadata>,

    pub headers: Option<std::collections::HashMap<String, String>>,

    pub transferred_bytes: u64,
}

#[async_trait]
pub trait TransferProtocol: Send + Sync + 'static {
    /// Get protocol type
    fn protocol_type(&self) -> ProtocolType;

    /// Get transfer mode
    fn transfer_mode(&self) -> TransferMode;

    /// Initialize transfer (Get metadata, create session, ...)
    async fn initialize(&self, ctx: &mut TransferContext) -> Result<()>;

    /// Get transferred bytes (Breakpoint resume)
    async fn get_progress(&self, ctx: &TransferContext) -> Result<u64>;

    /// Create transfer stream
    async fn create_stream(
        &self,
        ctx: &TransferContext
    ) -> Result<Pin<Box<dyn Stream<Item = Result<TransferChunk>> + Send>>>;

    /// The cleaning work after the transfer is completed
    async fn finalize(&self, ctx: &mut TransferContext) -> Result<()>;

    /// Cancel transfer
    async fn cancel(&self, ctx: &TransferContext) -> Result<()>;

    /// Verify whether the transfer can be restored
    async fn verify_resumable(&self, ctx: &TransferContext) -> Result<bool> {
        Ok(ctx.metadata.as_ref().map(|m| m.resumable).unwrap_or(false))
    }
}

/// Transfer chunk
#[derive(Debug)]
pub struct TransferChunk {
    pub data: Bytes,
    pub offset: u64,
    pub is_final: bool,
}

#[async_trait]
pub trait TransferProcessor: Send + Sync {
    /// Process data chunk
    async fn process_chunk(&mut self, chunk: TransferChunk) -> Result<()>;

    /// Finalize process
    async fn finalize(&mut self) -> Result<()>;
}

/// Transfer Task builder feature - Each protocol needs to implement its own builder
pub trait TransferTaskBuilder: Send + Sync {
    /// Build transfer context
    fn build_context(&self) -> Result<TransferContext>;

    /// Build protocol instance
    fn build_protocol(&self) -> Result<Box<dyn TransferProtocol>>;

    /// Build processor
    fn build_processor(&self) -> Pin<Box<dyn Future<Output = Result<Box<dyn TransferProcessor>>> + Send>>;
}

#[async_trait]
pub trait RateLimiter: Send + Sync {
    async fn acquire(&self, bytes: usize) -> Result<()>;
}

#[async_trait]
pub trait TransferHook: Send + Sync {
    async fn before_transfer(&self, ctx: &TransferContext) -> Result<()> {
        Ok(())
    }

    async fn before_chunk(&self, ctx: &TransferContext, chunk: &TransferChunk) -> Result<()> {
        Ok(())
    }

    async fn after_transfer(&self, ctx: &TransferContext) -> Result<()> {
        Ok(())
    }

    async fn on_error(&self, ctx: &TransferContext, error: &TransferError) -> Result<()> {
        Ok(())
    }
}

