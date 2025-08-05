use std::sync::Arc;
use super::types::{TransferId};
use super::traits::{TransferProtocol, TransferContext};

#[derive(Clone)]
pub struct TransferTask {
    pub id: TransferId,
    pub protocol: Arc<Box<dyn TransferProtocol>>,
    pub context: TransferContext,
}
