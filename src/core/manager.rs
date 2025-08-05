use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, broadcast, Semaphore, RwLock};
use tokio_util::sync::CancellationToken;
use dashmap::DashMap;
use crate::TransferConfig;
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
        let (event_tx, _) = broadcast::channel(1024);

        let manager = Self {
            command_tx,
            event_tx,
        };
        
        let pool_handle = tokio::spawn(
            WorkerPool::new(config).start()
        );

        let runner = ManagerExecutor {
            command_rx,
            cancellation_token: CancellationToken::new(),
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
}

impl ManagerExecutor {
    async fn run(mut self) {
        loop {
            tokio::select! {
                // 处理命令
                Some(command) = self.command_rx.recv() => {
                    self.handle_command(command);
                }
                // 取消信号
                _ = self.cancellation_token.cancelled() => {
                    break;
                }
            }
        }
    }

    fn handle_command(&self, command: ManagerCommand) {
        match command {
            ManagerCommand::AddTask { builder, reply } => {
                let result = self.add_task(builder);
                let _ = reply.send(result);
            }
            ManagerCommand::PauseTask { id, reply } => {
                let result = self.pause_task(id);
                let _ = reply.send(result);
            }
        }
    }

    fn add_task(&self, builder: Box<dyn TransferTaskBuilder>) -> Result<TransferId> {
        let context = builder.build_context();
        let protocol = builder.build_protocol();

        Ok(TransferId::new())
    }

    fn pause_task(&self, id: TransferId) -> Result<()> {
        Ok(())
    }
}

pub struct WorkerPool {
    config: TransferConfig,
    tasks: Arc<RwLock<DashMap<TransferId, tokio::task::JoinHandle<()>>>>,
    semaphore: Arc<Semaphore>,
}

impl WorkerPool {
    pub fn new(config: TransferConfig) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(config.concurrent)),
            tasks: Arc::new(RwLock::new(DashMap::new())),
            config,
        }
    }

    pub async fn start(mut self) {
        loop {
            let semaphore = self.semaphore.clone();
            let permit = semaphore.acquire_owned().await.unwrap();
            
            let id = TransferId::new();
            
            let id_clone = id.clone();
            let tasks_clone = self.tasks.clone();
            let handle = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                drop(permit);
                tasks_clone.write().await.remove(&id_clone);
            });
            
            self.tasks.write().await.insert(id, handle);
        }
    }

    
}
