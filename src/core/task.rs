use std::time::Instant;
use futures_util::SinkExt;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use super::queue::TaskQueue;
use super::{
    TransferId,
    TransferState,
    TransferEvent,
    TransferTask,
    TransferProtocolBuilder
};

pub enum ManagerCommand {
    AddTask {
        builder: Box<dyn TransferProtocolBuilder>,
        bytes_tx: mpsc::UnboundedSender<u64>,
        reply: oneshot::Sender<TransferId>,
    },
    PauseTask {
        id: TransferId,
        reply: oneshot::Sender<()>,
    },
    ResumeTask,
    CancelTask {
        id: TransferId,
        reply: oneshot::Sender<()>,
    },
}

struct CommandProcessor {
    queue: TaskQueue,
    command_rx: mpsc::UnboundedReceiver<ManagerCommand>,
    cancellation_token: CancellationToken,
}

impl CommandProcessor {
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                _ = self.cancellation_token.cancelled() => {
                    break;
                }
                Some(command) = self.command_rx.recv() => {
                    self.handle_command(command).await;
                }
            }
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
            ManagerCommand::PauseTask { reply, .. } => {
                let _ = reply.send(());
            }
            ManagerCommand::ResumeTask => {}
            ManagerCommand::CancelTask { id, reply } => {
                self.cancel_task(id).await;
                let _ = reply.send(());
            }
        }
    }

    async fn add_task(&mut self, builder: Box<dyn TransferProtocolBuilder>, bytes_tx: mpsc::UnboundedSender<u64>) -> TransferId {
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

        self.queue.add_task(task).await;

        // 如果有下一个任务则尝试执行
        self.queue.try_execute_next().await;

        transfer_id
    }

    async fn cancel_task(&mut self, id: TransferId) {
        let mut store_guard = self.queue.store.write().await;
        store_guard.cancel_task(&id);
    }

    async fn pause_task(&self) {
        
    }
    
    async fn resume_task(&self) {

    }
}

pub struct MangerWorker {
    cancellation_token: CancellationToken,
}

impl MangerWorker {
    pub fn new(command_rx: mpsc::UnboundedReceiver<ManagerCommand>, event_tx: broadcast::Sender<TransferEvent>) -> Self {
        let cancellation_token = CancellationToken::new();
        let command_processor = CommandProcessor {
            command_rx,
            cancellation_token: cancellation_token.clone(),
            queue: TaskQueue::new(1, event_tx),
        };
        tokio::spawn(command_processor.run());

        Self {
            cancellation_token,
        }
    }

    pub fn shutdown(&self) {
        self.cancellation_token.cancel();
    }
}


