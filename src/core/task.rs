use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;
use futures_util::SinkExt;
use tokio::sync::{Semaphore, mpsc, RwLock};
use tokio::task::JoinHandle;
use crate::TransferProtocol;
use super::{
    TransferId,
    TransferState,
    TransferProtocolBuilder
};

#[derive(Clone)]
pub enum TaskExecuteInnerEvent {
    Started { id: TransferId },
    Completed { id: TransferId },
    Failed { id: TransferId, reason: String },
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

struct TaskExecutor {
    semaphore: Arc<Semaphore>,
    event_tx: mpsc::UnboundedSender<TaskExecuteInnerEvent>
}

impl TaskExecutor {
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
                        reason: "Failed to acquire semaphore".to_string()
                    });
                    return;
                }
            };

            // Start
            let _ = event_tx.send(TaskExecuteInnerEvent::Started { id: task_id.clone() });

            // Initialize error
            let init_result = protocol.initialize(&mut context).await;
            if let Err(err) = init_result {
                let _ = event_tx.send(TaskExecuteInnerEvent::Failed {
                    id: task_id,
                    reason: err.to_string()
                });
                return;
            }

            // Execute result
            let execute_result = protocol.execute(&context, bytes_tx).await;
            match execute_result {
                Ok(_) => {
                    let _ = event_tx.send(TaskExecuteInnerEvent::Completed { id: task_id });
                }
                Err(err) => {
                    let _ = event_tx.send(TaskExecuteInnerEvent::Failed {
                        id: task_id,
                        reason: err.to_string()
                    });
                }
            }
        })
    }
}

#[derive(Default)]
struct TaskStore {
    tasks: HashMap<TransferId, TransferTask>,
    pending_queue: VecDeque<TransferId>,
    running_tasks: HashMap<TransferId, JoinHandle<()>>,
    completed_tasks: Vec<TransferId>,
    failed_tasks: Vec<(TransferId, String)>,
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
        self.failed_tasks.push((id.clone(), reason));
    }

    fn cancel_task(&mut self, id: &TransferId) {
        // self.update_state(id, TransferState::Cancelled);
        if let Some(task) = self.tasks.remove(id) {
            self.failed_tasks.retain(|(x, _)| x != id);
            self.pending_queue.retain(|x| x != id);
            if let Some(handle) = self.running_tasks.remove(id) {
                handle.abort();
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

pub struct QueueExecutor {
    store: TaskStore,
    executor: Arc<TaskExecutor>,
}

impl QueueExecutor {
    pub fn new(max_concurrent: usize, event_tx: mpsc::UnboundedSender<TaskExecuteInnerEvent>) -> Self {
        let executor = Arc::new(TaskExecutor {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            event_tx,
        });

        Self {
            store: TaskStore::new(),
            executor,
        }
    }

    async fn try_execute_next(&mut self) {
        if self.executor.semaphore.available_permits() == 0 {
            return;
        }

        if let Some(task_id) = self.store.get_next_pending() {
            if let Some(task) = self.store.get_task(&task_id) {
                let handle = self.executor.execute(task).await;
                self.store.add_running(task_id, handle);
            }
        }
    }

    pub async fn add_task(&mut self, task: TransferTask) {
        self.store.add_task(task);
        self.try_execute_next().await;
    }
}

pub struct TaskManager {
    pub queue_executor: Arc<RwLock<QueueExecutor>>,
    pub event_rx: mpsc::UnboundedReceiver<TaskExecuteInnerEvent>,
}

impl TaskManager {
    pub fn new(
        queue_executor: Arc<RwLock<QueueExecutor>>,
        event_rx: mpsc::UnboundedReceiver<TaskExecuteInnerEvent>
    ) -> Self {
        Self { queue_executor, event_rx }
    }
    
    pub async fn run(mut self) {
        while let Some(event) = self.event_rx.recv().await {
            self.handle_event(event).await;
        }
    }

    async fn handle_event(&mut self, event: TaskExecuteInnerEvent) {
        match event {
            TaskExecuteInnerEvent::Started { id } => {
                let mut queue_guard = self.queue_executor.write().await;
                queue_guard.store.update_state(&id, TransferState::Running);
            }
            TaskExecuteInnerEvent::Completed { id } => {
                println!("Completed: {:?}", id);
                let mut queue_guard = self.queue_executor.write().await;
                queue_guard.store.mark_completed(&id);
                queue_guard.try_execute_next().await;
            }
            TaskExecuteInnerEvent::Failed { id, reason } => {
                let mut queue_guard = self.queue_executor.write().await;
                queue_guard.store.mark_failed(&id, reason);
            }
        }
    }
}

