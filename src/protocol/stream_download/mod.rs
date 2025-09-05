use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedSender;
use crate::core::{
    Result,
    TransferError,
    TransferContext,
    TransferProtocolBuilder,
    TransferProtocol,
    TransferConfig,
};

pub struct StreamDownloadProtocol {
}

impl StreamDownloadProtocol {
    pub fn new() -> Self {
        todo!()
    }
}

#[async_trait]
impl TransferProtocol for StreamDownloadProtocol {
    async fn initialize(&self, ctx: &mut TransferContext) -> Result<()> {
        todo!()
    }

    async fn execute(&self, ctx: &TransferContext, progress_tx: Option<UnboundedSender<u64>>) -> Result<()> {
        todo!()
    }
}

pub struct StreamDownloadBuilder {
    config: TransferConfig
}

impl TransferProtocolBuilder for StreamDownloadBuilder {
    fn build_context(&self) -> TransferContext {
        TransferContext::new(self.config.source.clone())
    }

    fn build_protocol(&self) -> Box<dyn TransferProtocol> {
        Box::new(StreamDownloadProtocol::new())
    }
}
