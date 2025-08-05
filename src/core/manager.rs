use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, broadcast, Semaphore};
use tokio_util::sync::CancellationToken;
use crate::{TransferConfig, TransferTask, WorkerPool};
use crate::core::worker::TaskQueue;
use super::traits::TransferTaskBuilder;
use super::types::TransferId;
use super::errors::{Result, TransferError};

pub struct TransferManager {
    command_tx: mpsc::UnboundedSender<ManagerCommand>,
    event_tx: broadcast::Sender<()>,
}

impl TransferManager {
    pub fn new(config: TransferConfig) -> ManagerHandle {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, _) = broadcast::channel(128);
        let (task_tx, task_rx) = mpsc::channel(64);

        let cancellation_token = CancellationToken::new();
        let semaphore = Arc::new(Semaphore::new(config.concurrent));

        let manager = Self {
            command_tx,
            event_tx,
        };

        let pool_handle = tokio::spawn(
            WorkerPool::new(semaphore.clone()).start(task_rx)
        );

        let runner = ManagerExecutor {
            command_rx,
            cancellation_token,
            queue: TaskQueue::new(semaphore, task_tx),
        };

        let handle = tokio::spawn(runner.run());

        ManagerHandle {
            manager,
            handle,
            pool_handle,
        }
    }

    /// 订阅产生的事件
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.event_tx.subscribe()
    }

    pub async fn add(&self, builder: Box<dyn TransferTaskBuilder>) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(ManagerCommand::AddTask { builder, reply: tx })
            .map_err(|_| TransferError::ManagerShutdown)?;

        let _ = rx.await.map_err(|_| TransferError::ManagerShutdown)?;

        Ok(())
    }

    /// 暂停任务
    pub async fn pause(&self, id: TransferId) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(ManagerCommand::PauseTask { id, reply: reply_tx })
            .map_err(|_| TransferError::ManagerShutdown)?;

        reply_rx.await.map_err(|_| TransferError::ManagerShutdown)?
    }

    pub async fn cancel_all(&self) {}
}

pub struct ManagerHandle {
    pub pool_handle: tokio::task::JoinHandle<()>,
    pub manager: TransferManager,
    pub handle: tokio::task::JoinHandle<()>,
}

impl ManagerHandle {
    async fn shutdown(&self) {
        println!("Manager shutting down");

        // 停止处理队列
        self.handle.abort();

        // 取消所有任务
        self.manager.cancel_all().await;

        // 等待 Runner 结束
        self.pool_handle.abort();
    }
}

enum ManagerCommand {
    AddTask {
        builder: Box<dyn TransferTaskBuilder>,
        reply: oneshot::Sender<Result<TransferId>>
    },

    PauseTask {
        id: TransferId,
        reply: oneshot::Sender<Result<()>>
    }
}

pub struct ManagerExecutor {
    command_rx: mpsc::UnboundedReceiver<ManagerCommand>,
    cancellation_token: CancellationToken,
    queue: TaskQueue,
}

impl ManagerExecutor {
    async fn run(mut self) {
        loop {
            tokio::select! {
                // 处理命令
                Some(command) = self.command_rx.recv() => {
                    self.handle_command(command).await;
                }
                // 取消信号
                _ = self.cancellation_token.cancelled() => {
                    break;
                }
            }
        }
    }

    async fn handle_command(&self, command: ManagerCommand) {
        match command {
            ManagerCommand::AddTask { builder, reply } => {
                let result = self.add_task(builder).await;
                let _ = reply.send(result);
            }
            ManagerCommand::PauseTask { id, reply } => {
                let result = self.pause_task(id);
                let _ = reply.send(result);
            }
        }
    }

    async fn add_task(&self, builder: Box<dyn TransferTaskBuilder>) -> Result<TransferId> {
        let context = builder.build_context();
        let protocol = builder.build_protocol();

        let id = context.id.clone();
        let task = TransferTask {
            id: id.clone(),
            context,
            protocol: Arc::new(protocol),
        };
        
        self.queue.push(task).await;
        
        Ok(id)
    }

    fn pause_task(&self, id: TransferId) -> Result<()> {
        Ok(())
    }
}
