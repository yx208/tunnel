use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;
use futures_util::SinkExt;
use tokio::sync::{Semaphore, RwLock, broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use crate::{TransferEvent, TransferProtocol};
use super::{
    TransferId,
    TransferState,
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

#[derive(Clone)]
enum TaskExecuteInnerEvent {
    Started {
        id: TransferId,
        from: TransferState
    },
    
    Completed {
        id: TransferId,
        from: TransferState,
    },
    
    Failed {
        id: TransferId,
        reason: String,
        from: TransferState,
    },
}

pub struct TransferTask {
    pub id: TransferId,
    pub state: TransferState,
    pub builder: Box<dyn TransferProtocolBuilder>,
    pub bytes_tx: mpsc::UnboundedSender<u64>,
    pub created_at: Instant,
    pub started_at: Option<Instant>,
    pub completed_at: Option<Instant>,
}

#[derive(Default)]
struct TaskStore {
    tasks: HashMap<TransferId, TransferTask>,
    pending_queue: VecDeque<TransferId>,
    running_tasks: HashMap<TransferId, JoinHandle<()>>,
    completed_tasks: Vec<TransferId>,
    failed_tasks: HashMap<TransferId, String>,
}

impl TaskStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn add_task(&mut self, task: TransferTask) {
        let id = task.id.clone();
        self.tasks.insert(id.clone(), task);
        self.pending_queue.push_back(id);
    }

    fn has_next_task(&self) -> bool {
        self.pending_queue.len() > 0
    }

    fn get_next_pending(&mut self) -> Option<TransferId> {
        self.pending_queue.pop_front()
    }

    fn update_state(&mut self, id: &TransferId, state: TransferState) {
        if let Some(task) = self.tasks.get_mut(id) {
            task.state = state.clone();

            match state {
                TransferState::Running => {
                    task.started_at = Some(Instant::now());
                }
                TransferState::Completed | TransferState::Failed | TransferState::Cancelled => {
                    task.completed_at = Some(Instant::now());
                }
                _ => {}
            }
        }
    }

    fn mark_completed(&mut self, id: &TransferId) {
        self.running_tasks.remove(id);
        self.update_state(id, TransferState::Completed);
        self.completed_tasks.push(id.clone());
    }

    fn mark_failed(&mut self, id: &TransferId, reason: String) {
        self.running_tasks.remove(id);
        self.update_state(id, TransferState::Failed);
        self.failed_tasks.insert(id.clone(), reason);
    }

    fn cancel_task(&mut self, id: &TransferId) {
        if let Some(task) = self.tasks.remove(id) {
            match task.state {
                TransferState::Queued => {
                    self.pending_queue.retain(|x| x != id);
                }
                TransferState::Running => {
                    if let Some(handle) = self.running_tasks.remove(id) {
                        handle.abort();
                    }
                }
                TransferState::Completed => {
                    self.completed_tasks.retain(|x| x != id);
                }
                TransferState::Failed => {
                    self.failed_tasks.remove(id);
                }
                TransferState::Paused => {}
                TransferState::Cancelled => {}
            }
        }
    }

    fn add_running(&mut self, id: TransferId, handle: JoinHandle<()>) {
        self.update_state(&id, TransferState::Running);
        self.running_tasks.insert(id, handle);
    }

    fn get_task(&self, id: &TransferId) -> Option<&TransferTask> {
        self.tasks.get(id)
    }
}

pub struct TaskQueue {
    store: TaskStore,
    event_tx: mpsc::UnboundedSender<TaskExecuteInnerEvent>,
    semaphore: Arc<Semaphore>,
}

impl TaskQueue {
    pub fn new(max_concurrent: usize, event_tx: mpsc::UnboundedSender<TaskExecuteInnerEvent>) -> Self {
        Self {
            event_tx,
            store: TaskStore::new(),
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
        }
    }

    async fn try_execute_next(&mut self) {
        if self.semaphore.available_permits() == 0 {
            return;
        }

        if let Some(task_id) = self.store.get_next_pending() {
            if let Some(task) = self.store.get_task(&task_id) {
                let handle = self.execute(task).await;
                self.store.add_running(task_id, handle);
            }
        }
    }

    async fn execute(&self, task: &TransferTask) -> JoinHandle<()> {
        let mut event_tx = self.event_tx.clone();
        let semaphore = self.semaphore.clone();
        let task_id = task.id.clone();

        let mut context = task.builder.build_context();
        let protocol = task.builder.build_protocol();
        let bytes_tx = task.bytes_tx.clone();

        tokio::spawn(async move {
            let _permit = match semaphore.acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => {
                    let _ = event_tx.send(TaskExecuteInnerEvent::Failed {
                        id: task_id,
                        from: TransferState::Queued,
                        reason: "Failed to acquire semaphore".to_string()
                    });
                    return;
                }
            };

            // Start
            let _ = event_tx.send(TaskExecuteInnerEvent::Started {
                id: task_id.clone(),
                from: TransferState::Queued,
            });

            // Initialize error
            let init_result = protocol.initialize(&mut context).await;
            if let Err(err) = init_result {
                let _ = event_tx.send(TaskExecuteInnerEvent::Failed {
                    id: task_id,
                    reason: err.to_string(),
                    from: TransferState::Running,
                });
                return;
            }

            // Execute result
            let execute_result = protocol.execute(&context, bytes_tx).await;
            match execute_result {
                Ok(_) => {
                    let _ = event_tx.send(TaskExecuteInnerEvent::Completed {
                        id: task_id,
                        from: TransferState::Running,
                    });
                }
                Err(err) => {
                    let _ = event_tx.send(TaskExecuteInnerEvent::Failed {
                        id: task_id,
                        reason: err.to_string(),
                        from: TransferState::Running,
                    });
                }
            }
        })
    }

    pub async fn add_task(&mut self, task: TransferTask) {
        self.store.add_task(task);
        self.try_execute_next().await;
    }
}

pub struct TaskEventHandle {
    pub queue_executor: Arc<RwLock<TaskQueue>>,
    pub task_event_rx: mpsc::UnboundedReceiver<TaskExecuteInnerEvent>,
    pub event_tx: broadcast::Sender<TransferEvent>,
}

impl TaskEventHandle {
    pub async fn run(mut self) {
        while let Some(event) = self.task_event_rx.recv().await {
            self.handle_event(event).await;
        }
    }

    async fn handle_event(&mut self, event: TaskExecuteInnerEvent) {
        match event {
            TaskExecuteInnerEvent::Started { id, from } => {
                let mut queue_guard = self.queue_executor.write().await;
                queue_guard.store.update_state(&id, TransferState::Running);
                let _ = self.event_tx.send(TransferEvent::StateChanged {
                    id,
                    from,
                    to: TransferState::Running,
                    reason: None,
                });
            }
            TaskExecuteInnerEvent::Completed { id, from } => {
                let mut queue_guard = self.queue_executor.write().await;
                queue_guard.store.mark_completed(&id);

                let _ = self.event_tx.send(TransferEvent::StateChanged {
                    id,
                    reason: None,
                    from,
                    to: TransferState::Completed,
                });

                if queue_guard.store.has_next_task() {
                    queue_guard.try_execute_next().await;
                } else {
                    let _ = self.event_tx.send(TransferEvent::Finished);
                }
            }
            TaskExecuteInnerEvent::Failed { id, reason, from } => {
                let mut queue_guard = self.queue_executor.write().await;
                queue_guard.store.mark_failed(&id, reason.clone());
                let _ = self.event_tx.send(TransferEvent::StateChanged {
                    id,
                    from, 
                    to: TransferState::Failed,
                    reason: Some(reason),
                });
            }
        }
    }
}

struct TaskManagerWorker {
    queue: Arc<RwLock<TaskQueue>>,
    command_rx: mpsc::Receiver<ManagerCommand>,
}

impl TaskManagerWorker {
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
            ManagerCommand::PauseTask { id, reply } => {
                self.pause_task(id).await;
                let _ = reply.send(());
            }
            ManagerCommand::ResumeTask => {}
            ManagerCommand::CancelTask { id, reply } => {
                self.cancel_task(id).await;
                let _ = reply.send(());
            }
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
        let mut manager_guard = self.queue.write().await;
        manager_guard.add_task(task).await;

        transfer_id
    }

    async fn cancel_task(&self, id: TransferId) {
        let mut queue_guard = self.queue.write().await;
        queue_guard.store.cancel_task(&id);
    }

    async fn pause_task(&self, id: TransferId) {
        
    }
    
    async fn resume_task(&self, id: TransferId) {
        
    }
}

pub struct MangerWorkerHandle {
    worker_handle: JoinHandle<()>,
    manager_handle: JoinHandle<()>,
}

impl MangerWorkerHandle {
    pub fn new(command_rx: mpsc::Receiver<ManagerCommand>, event_tx: broadcast::Sender<TransferEvent>) -> Self {
        let (
            task_event_tx,
            task_event_rx,
        ) = mpsc::unbounded_channel();

        let queue = Arc::new(RwLock::new(
            TaskQueue::new(1, task_event_tx)
        ));

        let manager_worker = TaskManagerWorker { queue: queue.clone(), command_rx };
        let manager_handle = tokio::spawn(manager_worker.run());

        let manager = TaskEventHandle {
            queue_executor: queue,
            task_event_rx,
            event_tx,
        };
        let worker_handle = tokio::spawn(manager.run());

        Self {
            worker_handle,
            manager_handle
        }
    }

    pub fn shutdown(&self) {
        self.manager_handle.abort();
        self.worker_handle.abort();
    }
}


