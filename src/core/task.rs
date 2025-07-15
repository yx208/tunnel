use std::sync::Arc;
use tokio::sync::RwLock;
use crate::core::types::TransferState::{Failed, Queued};
use super::traits::{TransferContext, TransferProtocol};
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
    pub context: Arc<RwLock<TransferContext>>,
    pub stats: Arc<RwLock<TransferStats>>,
    pub options: TransferOptions,
    pub protocol: Arc<Box<dyn TransferProtocol>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub started_at: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
    pub completed_at: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
    pub error: Arc<RwLock<Option<String>>>,
}

impl TransferTask {
    pub fn new(
        context: TransferContext,
        protocol: Box<dyn TransferProtocol>,
        options: TransferOptions,
    ) -> Self {
        let total_bytes = context.metadata.as_ref().and_then(|m| m.total_size);

        Self {
            id: context.id,
            state: Arc::new(RwLock::new(TransferState::Queued)),
            context: Arc::new(RwLock::new(context)),
            stats: Arc::new(RwLock::new(TransferStats::new(total_bytes))),
            options,
            protocol: Arc::new(protocol),
            created_at: chrono::Utc::now(),
            started_at: Arc::new(RwLock::new(None)),
            completed_at: Arc::new(RwLock::new(None)),
            error: Arc::new(RwLock::new(None)),
        }
    }

    pub fn is_valid_transition(from: TransferState, to: TransferState) -> bool {
        use TransferState::*;

        match (from, to) {
            // any to cancel
            (_, Cancelled) => true,

            // retry
            (Failed, Queued) => true,

            // terminal state cannot be converted
            (Completed, _) | (Failed, _) | (Cancelled, _) => false,

            // normal process
            (Queued, Preparing) => true,
            (Preparing, Transferring) => true,
            (Transferring, Completed) => true,

            // pause/resume
            (Transferring, Paused) => true,
            (Paused, Transferring) => true,
            (Queued, Paused) => true,
            (Paused, Queued) => true,
            
            // fail
            (Preparing, Failed) | (Transferring, Failed) => true,

            _ => false
        }
    }
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
