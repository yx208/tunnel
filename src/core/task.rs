use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use super::{Result, TransferError};
use super::types::TransferState::{Failed, Queued};
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

    pub async fn set_state(&self, new_state: TransferState) -> Result<TransferState> {
        let mut state = self.state.write().await;
        let old_state = *state;

        if !Self::is_valid_transition(old_state, new_state) {
            return Err(TransferError::invalid_state(
                format!("{:?}", new_state),
                format!("{:?}", old_state),
            ));
        }

        *state = new_state;

         match new_state {
             TransferState::Transferring => {
                 *self.started_at.write().await = Some(chrono::Utc::now());
             }
             TransferState::Completed | TransferState::Failed | TransferState::Cancelled => {
                 *self.completed_at.write().await = Some(chrono::Utc::now());
                 let mut stats = self.stats.write().await;
                 stats.end_time = Some(Instant::now());
             }
             _ => {}
         }

        Ok(old_state)
    }

    pub async fn set_error(&self, error: Option<String>) {
        *self.error.write().await = error;
    }

    pub async fn set_current_speed(&self, speed: f64) {
        let mut stats = self.stats.write().await;
        stats.current_speed = speed;
    }

    pub async fn increment_retry_count(&self) {
        let mut stats = self.stats.write().await;
        stats.retry_count += 1;
    }

    pub async fn update_progress(&self, bytes_transferred: u64) {
        let mut stats = self.stats.write().await;
        stats.bytes_transferred = bytes_transferred;
        
        let elapsed = stats.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            stats.average_speed = bytes_transferred as f64 / elapsed;
            
            if let Some(total) = stats.total_bytes {
                let remaining = total.saturating_sub(bytes_transferred);
                if stats.current_speed > 0.0 && remaining > 0 {
                    stats.eta = Some(std::time::Duration::from_secs_f64(
                        remaining as f64 / stats.current_speed
                    ));
                }
            }
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
