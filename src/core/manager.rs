use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, broadcast, oneshot, Semaphore};

use super::progress::{ProgressAggregator, ProgressTracker};
use super::traits::TransferTaskBuilder;
use super::task::{TaskControl, TaskQueue, TransferTask};
use super::types::{TransferConfig, TransferEvent, TransferId};
use super::errors::Result;

pub struct TransferManger {
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

impl TransferManger {
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

        todo!()
    }
}


pub struct TransferMangerHandle {

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_manager() {
    }
}

