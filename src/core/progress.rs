use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::{mpsc, RwLock};
use super::types::{TransferId, TransferStats};

#[async_trait]
pub trait ProgressTracker: Send + Sync {
    async fn register_task(&self, id: TransferId, total_size: u64);
    async fn unregister_task(&self, id: TransferId);
    async fn update_progress(&self, id: TransferId, bytes_transferred: u64);
    async fn get_progress(&self, id: TransferId) -> Option<TransferStats>;
    async fn get_all_progress(&self) -> HashMap<TransferId, TransferStats>;
}

pub struct SimpleProgressTracker {
    tasks: Arc<RwLock<HashMap<TransferId, TransferStats>>>,
}

pub struct ProgressAggregator {}

impl ProgressAggregator {
    pub fn new(
        batch_tx: mpsc::UnboundedSender<Vec<(TransferId, TransferStats)>>,
        update_interval: Duration,
    ) -> Self {
        Self {}
    }
}

#[async_trait]
impl ProgressTracker for ProgressAggregator {
    async fn register_task(&self, id: TransferId, total_size: u64) {
        todo!()
    }

    async fn unregister_task(&self, id: TransferId) {
        todo!()
    }

    async fn update_progress(&self, id: TransferId, bytes_transferred: u64) {
        todo!()
    }

    async fn get_progress(&self, id: TransferId) -> Option<TransferStats> {
        todo!()
    }

    async fn get_all_progress(&self) -> HashMap<TransferId, TransferStats> {
        todo!()
    }
}


