use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::task::JoinHandle;
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
        reply: oneshot::Sender<Result<()>>,
    },
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
            ManagerCommand::AddTask { builder, reply } => {
                self.add_task(builder).await;
                let _ = reply.send(Ok(()));
            }
        }
    }

    async fn add_task(&self, builder: Box<dyn TransferProtocolBuilder>) {
        let task = TransferTask::new(builder);
        let mut manager_guard = self.executor.write().await;
        manager_guard.add_task(task).await;

        // manager_guard.push_task(task).await;
    }
}

pub struct TunnelScheduler {
    command_tx: mpsc::Sender<ManagerCommand>,
    worker_handle: JoinHandle<()>,
    manager_handle: JoinHandle<()>,
}

impl TunnelScheduler {
    pub fn new() -> Self {
        let (command_tx, command_rx) = mpsc::channel(64);
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let executor = Arc::new(RwLock::new(
            QueueExecutor::new(3, event_tx)
        ));
        
        let worker = ManagerWorker { executor: executor.clone(), command_rx };
        let worker_handle = tokio::spawn(worker.run());
        
        let manager = TaskManager::new(executor.clone(), event_rx);
        let manager_handle = tokio::spawn(manager.run());

        Self {
            command_tx,
            manager_handle,
            worker_handle
        }
    }

    pub async fn add_task(&self, builder: Box<dyn TransferProtocolBuilder>) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(ManagerCommand::AddTask { builder, reply: reply_tx })
            .await
            .map_err(|_| TransferError::ManagerShutdown)?;

        reply_rx.await.map_err(|_| TransferError::ManagerShutdown)?
    }
}
