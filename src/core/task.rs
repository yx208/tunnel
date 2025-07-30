use super::traits::TransferProtocol;

pub struct TransferTask {
    protocol: Box<dyn TransferProtocol>
}
