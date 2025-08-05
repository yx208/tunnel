use super::types::{TransferId};
use super::traits::{TransferProtocol, TransferContext};

pub struct TransferTask {
    id: TransferId,
    protocol: Box<dyn TransferProtocol>,
    context: TransferContext,
}
