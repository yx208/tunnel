use std::cmp::PartialEq;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, RwLock, Semaphore};
use tokio::task::JoinHandle;
use crate::{Result, TransferError, TransferId, TransferProtocolBuilder};

#[derive(Clone, Eq, PartialEq)]
pub enum TaskStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone)]
struct TransferTask {
    id: TransferId,
    status: TaskStatus,
    builder: Arc<Box<dyn TransferProtocolBuilder>>,
}

struct TaskManager {
    task_tx: mpsc::UnboundedSender<TransferId>,
    completed_tx: mpsc::UnboundedSender<TransferId>,
    tasks: HashMap<TransferId, TransferTask>,
    running_tasks: HashMap<TransferId, JoinHandle<Result<()>>>,
    pending_tasks: VecDeque<TransferId>,
    semaphore: Arc<Semaphore>,
}

impl TaskManager {
    fn new(
        semaphore: Arc<Semaphore>,
        completed_tx: mpsc::UnboundedSender<TransferId>,
        task_tx: mpsc::UnboundedSender<TransferId>
    ) -> Self {
        Self {
            task_tx,
            completed_tx,
            semaphore,
            running_tasks: HashMap::new(),
            pending_tasks: VecDeque::new(),
            tasks: HashMap::new(),
        }
    }

    fn remove_running_task(&mut self, id: TransferId) {
        self.running_tasks.remove(&id);
    }

    async fn push_task(&mut self, task: TransferTask) {
        let task_id = task.id.clone();
        if task.status == TaskStatus::Pending && self.semaphore.available_permits() >= 1 {
            self.tasks.insert(task_id.clone(), task);
            self.execute(task_id).await;
        } else {
            self.pending_tasks.push_front(task_id.clone());
            self.tasks.insert(task_id.clone(), task);
        }
    }

    fn has_next_task(&self) -> bool {
        !self.pending_tasks.is_empty()
    }

    async fn execute(&mut self, transfer_id: TransferId) {
        let task = self.tasks.get(&transfer_id).unwrap();
        let mut context = task.builder.build_context().await;
        let protocol = task.builder.build_protocol().await;

        let task_id_clone = transfer_id.clone();
        let completed_tx = self.completed_tx.clone();
        let semaphore = self.semaphore.clone();
        let handle = tokio::spawn(async move {
            let permit = semaphore.acquire_owned().await;
            protocol.initialize(&mut context).await?;
            protocol.execute(&context, None).await?;
            drop(permit);
            let _ = completed_tx.send(task_id_clone);
            Ok(())
        });

        self.running_tasks.insert(transfer_id, handle);
    }

    async fn execute_next(&mut self) {
        if self.pending_tasks.is_empty() {
            return;
        }

        let next_id = self.pending_tasks.pop_back().unwrap();
        self.execute(next_id).await;
    }
}

struct TaskWorker {
    task_rx: mpsc::UnboundedReceiver<TransferId>,
    completed_rx: mpsc::UnboundedReceiver<TransferId>,
    manager: Arc<RwLock<TaskManager>>,
}

impl TaskWorker {
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                result = self.task_rx.recv() => {
                    if let Some(transfer_id) = result {
                        let mut manager_guard = self.manager.write().await;
                        manager_guard.execute(transfer_id).await;
                    }
                }
                Some(transfer_id) = self.completed_rx.recv() => {
                    println!("Task completed: {:?}", transfer_id);

                    let mut manager_guard = self.manager.write().await;
                    manager_guard.remove_running_task(transfer_id);

                    if manager_guard.has_next_task() {
                        manager_guard.execute_next().await;
                    } else {
                        println!("All task completed");
                        // todo!("Send task finished event");
                    }
                }
            }
        }
    }
}

enum ManagerCommand {
    AddTask {
        builder: Box<dyn TransferProtocolBuilder>,
        reply: oneshot::Sender<Result<()>>,
    },
}

struct ManagerWorker {
    manager: Arc<RwLock<TaskManager>>,
    command_rx: mpsc::Receiver<ManagerCommand>,
}

impl ManagerWorker {
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                Some(result) = self.command_rx.recv() => {
                    self.handle_command(result).await;
                }
            }
        }
    }

    pub async fn handle_command(&mut self, command: ManagerCommand) {
        match command {
            ManagerCommand::AddTask { builder, reply } => {
                let task = TransferTask {
                    id: TransferId::new(),
                    status: TaskStatus::Pending,
                    builder: Arc::new(builder),
                };
                let mut manager_guard = self.manager.write().await;
                manager_guard.push_task(task).await;
                let _ = reply.send(Ok(()));
            }
        }
    }
}

pub struct TunnelScheduler {
    command_tx: mpsc::Sender<ManagerCommand>,
    task_worker_handle: JoinHandle<()>,
    manager_worker_handle: JoinHandle<()>,
}

impl TunnelScheduler {
    pub fn new() -> Self {
        let (completed_tx, completed_rx) = mpsc::unbounded_channel();
        let (task_tx, task_rx) = mpsc::unbounded_channel();
        let (command_tx, command_rx) = mpsc::channel(64);

        let manager = Arc::new(RwLock::new(
            TaskManager::new(
                Arc::new(Semaphore::new(1)),
                completed_tx,
                task_tx
            )
        ));

        let worker = TaskWorker {
            manager: manager.clone(),
            completed_rx,
            task_rx,
        };
        let task_worker_handle = tokio::spawn(worker.run());

        let manager_worker = ManagerWorker {
            manager,
            command_rx,
        };
        let manager_worker_handle = tokio::spawn(manager_worker.run());

        Self {
            command_tx,
            task_worker_handle,
            manager_worker_handle,
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
