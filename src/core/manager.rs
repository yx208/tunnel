use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, broadcast, Semaphore, RwLock};
use tokio_util::sync::CancellationToken;
use crate::{TransferConfig, TransferTask};
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
        let (worker_tx, worker_rx) = mpsc::unbounded_channel();

        let manager = Self {
            command_tx,
            event_tx,
        };

        let worker_pool = WorkerPool::new(config.concurrent);
        let worker_handle = tokio::spawn(worker_pool.run(worker_rx));

        let inner = ManagerInner {
            semaphore: Arc::new(Semaphore::new(config.concurrent)),
            tasks: Arc::new(RwLock::new(Vec::new())),
            active_tasks: Vec::new(),
            config,
        };

        let runner = ManagerExecutor {
            inner,
            command_rx,
            cancellation_token: CancellationToken::new(),
        };
        let handle = tokio::spawn(runner.run());

        ManagerHandle {
            manager,
            handle,
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

struct ManagerInner {
    config: TransferConfig,
    semaphore: Arc<Semaphore>,
    tasks: Arc<RwLock<Vec<TransferTask>>>,
    active_tasks: Vec<TransferId>,
}

pub struct ManagerExecutor {
    inner: ManagerInner,
    command_rx: mpsc::UnboundedReceiver<ManagerCommand>,
    cancellation_token: CancellationToken,
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

        {
            let mut guard = self.inner.tasks.write().await;
            guard.push(task);
        }

        // 有可用线程
        if self.inner.semaphore.available_permits() != 0 {
            // self.worker_tx.send(task);
        }

        Ok(id)
    }

    fn pause_task(&self, id: TransferId) -> Result<()> {
        Ok(())
    }

    fn cancel_task(&self) -> Result<()> {
        Ok(())
    }
}

struct WorkerTask {
    task: TransferTask,
    completion_tx: oneshot::Sender<()>,
}

struct WorkerPool {
    workers: Vec<WorkerTask>
}

impl WorkerPool {
    fn new(size: usize) -> Self {
        Self { workers: Vec::with_capacity(size) }
    }

    async fn run(mut self, mut worker_rx: mpsc::UnboundedReceiver<WorkerTask>) {
        while let Some(worker_task) = worker_rx.recv().await {
            println!("Worker task received");
        }
    }

    async fn shutdown(&self) {
        for worker in &self.workers {
            // worker.abort();
        }
    }
}

/*
worker
    + notify
    + worker_threads

    # add -> trigger
    #


 */

