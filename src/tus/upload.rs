use async_trait::async_trait;
use crate::core::{TransferProtocol, TransferTaskBuilder, Result};
use super::types::TusConfig;

pub struct TusProtocol {
    config: TusConfig,
}

impl TusProtocol {
    pub fn new(config: TusConfig) -> Self {
        Self { config }
    }

    pub fn create_upload() {

    }
}

#[async_trait]
impl TransferProtocol for TusProtocol {
    async fn pause(&self) -> Result<()> {
        Ok(())
    }
}

pub struct TusTaskBuilder {}

impl TransferTaskBuilder for TusTaskBuilder {
    fn build_protocol(&self) -> Box<dyn TransferProtocol> {
        todo!()
    }
}
