use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tokio::sync::mpsc;
use super::traits::TransferProcessor;
use super::progress::ProgressTracker;
use super::task::TransferTask;
use super::types::TransferConfig;

pub struct WorkerPool {
    workers: Vec<WorkerHandle>,
    config: TransferConfig,
    cancellation_token: CancellationToken,
}

impl WorkerPool {
    pub fn new(size: usize, config: TransferConfig) -> Self {
        let cancellation_token = CancellationToken::new();

        Self {
            workers: Vec::with_capacity(size),
            config,
            cancellation_token
        }
    }

    pub fn start(
        mut self,
        task_rx: mpsc::Receiver<WorkerTask>,
        progress_tracker: Option<Arc<dyn ProgressTracker>>
    ) -> Self {
        self
    }
}

struct WorkerHandle {
    id: usize,
    handle: tokio::task::JoinHandle<()>,
}

pub(crate) struct WorkerTask {
    pub task: TransferTask,
     pub processor: Box<dyn TransferProcessor>,
}
