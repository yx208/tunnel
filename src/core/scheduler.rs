use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, mpsc, oneshot, RwLock};
use tokio::task::JoinHandle;
use crate::progress::ProgressAggregator;
use crate::{TransferEvent, TransferId, TransferState};
use super::{
    Result,
    TransferError,
    TransferTask,
    TaskManager,
    QueueExecutor,
    TransferProtocolBuilder,
};

enum ManagerCommand {
    AddTask {
        builder: Box<dyn TransferProtocolBuilder>,
        bytes_tx: mpsc::UnboundedSender<u64>,
        reply: oneshot::Sender<TransferId>,
    },
    PauseTask,
    ResumeTask,
    CancelTask,
}

struct ManagerWorker {
    executor: Arc<RwLock<QueueExecutor>>,
    command_rx: mpsc::Receiver<ManagerCommand>,
}

impl ManagerWorker {
    pub async fn run(mut self) {
        while let Some(command) = self.command_rx.recv().await {
            self.handle_command(command).await;
        }
    }

    pub async fn handle_command(&mut self, command: ManagerCommand) {
        match command {
            ManagerCommand::AddTask {
                builder,
                bytes_tx,
                reply,
            } => {
                let id = self.add_task(builder, bytes_tx).await;
                let _ = reply.send(id);
            }
            ManagerCommand::PauseTask => {}
            ManagerCommand::ResumeTask => {}
            ManagerCommand::CancelTask => {}
        }
    }

    async fn add_task(&self, builder: Box<dyn TransferProtocolBuilder>, bytes_tx: mpsc::UnboundedSender<u64>) -> TransferId {
        let transfer_id = TransferId::new();
        let task = TransferTask {
            builder,
            bytes_tx,
            id: transfer_id.clone(),
            state: TransferState::Queued,
            created_at: Instant::now(),
            started_at: None,
            completed_at: None,
        };
        let mut manager_guard = self.executor.write().await;
        manager_guard.add_task(task).await;

        transfer_id
    }
}

pub struct TunnelScheduler {
    command_tx: mpsc::Sender<ManagerCommand>,
    worker_handle: JoinHandle<()>,
    manager_handle: JoinHandle<()>,
    aggregator: ProgressAggregator,
    event_tx: broadcast::Sender<TransferEvent>,
}

impl TunnelScheduler {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(128);
        let (command_tx, command_rx) = mpsc::channel(64);
        let (executor, manager) = create_manager_and_executor();

        let manager_handle = tokio::spawn(manager.run());
        
        let worker = ManagerWorker { executor: executor.clone(), command_rx };
        let worker_handle = tokio::spawn(worker.run());

        let aggregator = ProgressAggregator::new(event_tx.clone())
            .enable_report();

        Self {
            command_tx,
            manager_handle,
            worker_handle,
            aggregator,
            event_tx
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TransferEvent> {
        self.event_tx.subscribe()
    }

    pub async fn add_task(&self, builder: Box<dyn TransferProtocolBuilder>) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let (bytes_tx, bytes_rx) = mpsc::unbounded_channel();

        self.command_tx
            .send(ManagerCommand::AddTask { builder, reply: reply_tx, bytes_tx })
            .await
            .map_err(|_| TransferError::ManagerShutdown)?;

        let id = reply_rx
            .await
            .map_err(|_| TransferError::ManagerShutdown)?;

        self.aggregator.registry_task(id, bytes_rx).await;

        Ok(())
    }

    pub async fn cancel_task(&self, id: TransferId) -> Result<()> {
        self.aggregator.unregister_task(&id).await;
        Ok(())
    }
}

fn create_manager_and_executor() -> (Arc<RwLock<QueueExecutor>>, TaskManager) {
    let (event_tx, event_rx) = mpsc::unbounded_channel();

    let executor = Arc::new(RwLock::new(
        QueueExecutor::new(3, event_tx)
    ));

    let manager = TaskManager::new(executor.clone(), event_rx);

    (executor, manager)
}
