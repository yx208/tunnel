use std::collections::HashMap;

use tokio::sync::{mpsc, oneshot};

use super::types::TransferId;
use super::task::TransferTask;
use super::errors::Result;

pub struct TransferManager {
    command_tx: mpsc::UnboundedSender<MangerCommand>,
    tasks: HashMap<TransferId, TransferTask>,
}

impl TransferManager {
    pub fn new() -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();

        Self {
            command_tx,
            tasks: HashMap::new(),
        }
    }

    pub fn subscribe() {

    }
}

pub enum MangerCommand {
    AddTask {
        reply: oneshot::Sender<Result<TransferId>>
    }
}
