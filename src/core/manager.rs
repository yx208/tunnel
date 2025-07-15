use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, broadcast, oneshot, Semaphore};
use tokio_util::sync::CancellationToken;
use super::worker::{WorkerPool, WorkerTask};
use super::progress::{ProgressAggregator, ProgressTracker};
use super::traits::TransferTaskBuilder;
use super::task::{TaskControl, TaskQueue, TransferTask};
use super::types::{TransferConfig, TransferEvent, TransferId};
use super::errors::{Result, TransferError};

#[derive(Clone)]
pub struct TransferManager {
    /// Inner state
    inner: Arc<RwLock<ManagerInner>>,

    /// Command send channel
    command_tx: mpsc::Sender<ManagerCommand>,

    /// event
    event_tx: broadcast::Sender<TransferEvent>,
}

struct ManagerInner {
    tasks: HashMap<TransferId, TransferTask>,
    /// Task control channels
    task_controls: HashMap<TransferId, mpsc::Sender<TaskControl>>,
    active_tasks: HashMap<TransferId, tokio::task::JoinHandle<()>>,
    queue: TaskQueue,
    config: TransferConfig,
    semaphore: Arc<Semaphore>,
}

enum ManagerCommand {
    AddTask {
        builder: Box<dyn TransferTaskBuilder + Send>,
        reply: oneshot::Sender<Result<TransferId>>,
    }
}

impl TransferManager {
    pub fn new(config: TransferConfig) -> TransferMangerHandle {
        let (command_tx, command_rx) = mpsc::channel(100);
        let (event_tx, _) = broadcast::channel(1024);

        let semaphore = Arc::new(Semaphore::new(config.max_concurrent));

        let inner = Arc::new(RwLock::new(ManagerInner {
            tasks: HashMap::new(),
            task_controls: HashMap::new(),
            active_tasks: HashMap::new(),
            queue: TaskQueue::new(),
            config: config.clone(),
            semaphore: semaphore.clone(),
        }));

        let manager = Self {
            inner: inner.clone(),
            command_tx,
            event_tx: event_tx.clone(),
        };

        let progress_aggregator = if config.batch_progress {
            let (batch_tx, mut batch_rx) = mpsc::unbounded_channel();
            let event_tx_clone = event_tx.clone();

            tokio::spawn(async move {
                while let Some(batch) = batch_rx.recv().await {
                    let _ = event_tx_clone.send(TransferEvent::BatchProgress {
                        updates: batch
                    });
                }

                println!("Task progress is end");
            });

            Some(Arc::new(ProgressAggregator::new(
                batch_tx,
                config.progress_interval,
            )) as Arc<dyn ProgressTracker>)
        } else {
            None
        };

        let (worker_tx, worker_rx) = mpsc::channel(config.max_concurrent * 2);
        let worker_pool = WorkerPool::new(config.max_concurrent, config.clone())
            .start(worker_rx, progress_aggregator.clone());

        let manager_task = ManagerTask {
            inner: inner.clone(),
            command_rx,
            worker_tx,
            event_tx: event_tx.clone(),
            progress_tracker: progress_aggregator,
            cancellation_token: CancellationToken::new(),
        };

        let handle = tokio::spawn(manager_task.run());

        TransferMangerHandle {
            manager,
            handle,
            worker_pool: Some(worker_pool),
        }
    }

    pub async fn add_task(&self, builder: Box<dyn TransferTaskBuilder + Send>) -> Result<TransferId> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(ManagerCommand::AddTask { builder, reply: reply_tx })
            .await
            .map_err(|_| TransferError::ManagerShutdown)?;
        
        reply_rx.await.map_err(|_| TransferError::ManagerShutdown)?
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TransferEvent> {
        self.event_tx.subscribe()
    }
}

pub struct TransferMangerHandle {
    pub manager: TransferManager,
    handle: tokio::task::JoinHandle<()>,
    worker_pool: Option<WorkerPool>,
}

struct ManagerTask {
    inner: Arc<RwLock<ManagerInner>>,
    command_rx: mpsc::Receiver<ManagerCommand>,
    event_tx: broadcast::Sender<TransferEvent>,
    worker_tx: mpsc::Sender<WorkerTask>,
    progress_tracker: Option<Arc<dyn ProgressTracker>>,
    cancellation_token: CancellationToken,
}

impl ManagerTask {
    async fn run(mut self) {
        loop {
            tokio::select! {
                Some(command) = self.command_rx.recv() => {

                }

                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                    self.check_queue().await;
                }

                _ = self.cancellation_token.cancelled() => {
                    break;
                }
            }
        }
    }

    async fn check_queue(&mut self) {

    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_manager() {
    }
}

