use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::TransferContext;
use super::errors::Result;

#[async_trait]
pub trait TransferProtocol: Send + 'static {
    async fn initialize(&self, ctx: &mut TransferContext) -> Result<()>;
    
    async fn execute(
        &self,
        ctx: &TransferContext,
        progress_tx: Option<mpsc::UnboundedSender<u64>>
    ) -> Result<()>;
}

pub trait TransferProtocolBuilder: Send + Sync + 'static {
    fn build_context(&self) -> TransferContext;
    
    fn build_protocol(&self) -> Box<dyn TransferProtocol>;
}