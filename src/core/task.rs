use std::sync::Arc;
use tokio::sync::RwLock;
use super::traits::TransferProtocol;
use super::types::{
    TransferId,
    TransferOptions,
    TransferState,
    TransferStats
};

#[derive(Clone)]
pub struct TransferTask {
    pub id: TransferId,
    pub state: Arc<RwLock<TransferState>>,
    pub context: Arc<RwLock<TransferStats>>,
    pub stats: Arc<RwLock<TransferStats>>,
    pub options: TransferOptions,
    pub protocol: Arc<Box<dyn TransferProtocol>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub started_at: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
    pub completed_at: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
    pub error: Arc<RwLock<Option<String>>>,
}

impl TransferTask {

}


#[derive(Debug)]
pub(crate) enum TaskControl {
    Pause,
    Resume,
    Cancel,
}

pub struct TaskQueue {
    tasks: Arc<RwLock<Vec<TransferTask>>>,
}

impl TaskQueue {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(Vec::new())),
        }
    }
}
