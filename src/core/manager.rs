use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use tokio::select;
use tokio::time::sleep;
use tokio::sync::{mpsc, oneshot, broadcast, RwLock};
use tokio_util::sync::CancellationToken;

use super::traits::TransferTaskBuilder;
use super::types::TransferId;
use super::task::TransferTask;
use super::errors::{Result, TransferError};

pub struct TransferManager {
    command_tx: mpsc::UnboundedSender<ManagerCommand>,
    tasks: HashMap<TransferId, TransferTask>,
    event_tx: broadcast::Sender<()>,
}

impl TransferManager {
    pub fn new() -> ManagerHandle {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, _) = broadcast::channel(1024);

        let manager = Self {
            command_tx,
            event_tx,
            tasks: HashMap::new(),
        };

        let runner = ManagerRunner {
            queue: Arc::new(RwLock::new(TaskQueue {})),
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

    /// 暂停任务
    pub async fn pause(&self, id: TransferId) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(ManagerCommand::PauseTask { id, reply: reply_tx })
            .map_err(|_| TransferError::ManagerShutdown)?;

        reply_rx.await.map_err(|_| TransferError::ManagerShutdown)?
    }
    
    pub async fn cancel_all(&self) {
        
    }
}

pub struct ManagerHandle {
    manager: TransferManager,
    handle: tokio::task::JoinHandle<()>,
}

impl ManagerHandle {
    async fn shutdown(&self) {
        println!("Manager shutting down");
        
        // 取消所有任务
        self.manager.cancel_all().await;
        
        // 等待 Runner 结束
        self.handle.abort();
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

pub struct ManagerRunner {
    queue: Arc<RwLock<TaskQueue>>,
    command_rx: mpsc::UnboundedReceiver<ManagerCommand>,
    cancellation_token: CancellationToken,
}

impl ManagerRunner {
    async fn run(mut self) {
        loop {
            select! {
                // 处理命令
                Some(command) = self.command_rx.recv() => {
                    self.handle_command(command).await;
                }

                // 定时检查任务
                _ = sleep(Duration::from_millis(1000)) => {
                    self.check_queue();
                }

                // 取消信号
                _ = self.cancellation_token.cancelled() => {
                    break;
                }
            }
        }
    }

    fn check_queue(&self) {

    }

    async fn handle_command(&self, command: ManagerCommand) {
        match command {
            ManagerCommand::AddTask { builder, reply } => {
                let result = self.add_task(builder);
                let _ = reply.send(result);
            }
            ManagerCommand::PauseTask { id, reply } => {
                let result = self.pause_task(id).await;
                let _ = reply.send(result);
            }
        }
    }

    fn add_task(&self, builder: Box<dyn TransferTaskBuilder>) -> Result<TransferId> {
        builder.build_protocol();
        Ok(TransferId::new())
    }

    async fn pause_task(&self, id: TransferId) -> Result<()> {
        Ok(())
    }
}

struct TaskQueue {

}
