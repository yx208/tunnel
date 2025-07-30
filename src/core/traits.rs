use async_trait::async_trait;
use super::errors::Result;

#[async_trait]
pub trait TransferProtocol {
    async fn pause(&self) -> Result<()>;
}

pub trait TransferTaskBuilder: Send + Sync {
    fn build_protocol(&self) -> Box<dyn TransferProtocol>;
}
