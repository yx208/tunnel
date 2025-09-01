use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock, Semaphore};
use tokio::task::JoinHandle;
use crate::{TransferEvent, TransferId, TransferState, TransferTask};
use super::store::TaskStore;

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

pub struct TaskEventProcessor {
    pub store: Arc<RwLock<TaskStore>>,
    pub task_event_rx: mpsc::UnboundedReceiver<TaskExecuteInnerEvent>,
    pub event_tx: broadcast::Sender<TransferEvent>,
}

impl TaskEventProcessor {
    pub async fn run(mut self) {
        while let Some(event) = self.task_event_rx.recv().await {
            self.handle_event(event).await;
        }
    }

    async fn handle_event(&mut self, event: TaskExecuteInnerEvent) {
        let mut store_guard = self.store.write().await;
        match event {
            TaskExecuteInnerEvent::Started { id, from } => {
                store_guard.update_state(&id, TransferState::Running);
                let _ = self.event_tx.send(TransferEvent::StateChanged {
                    id,
                    from,
                    to: TransferState::Running,
                    reason: None,
                });
            }
            TaskExecuteInnerEvent::Completed { id, from } => {
                store_guard.mark_completed(&id);
                let _ = self.event_tx.send(TransferEvent::StateChanged {
                    id,
                    reason: None,
                    from,
                    to: TransferState::Completed,
                });
            }
            TaskExecuteInnerEvent::Failed { id, reason, from } => {
                store_guard.mark_failed(&id, reason.clone());
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

pub struct TaskQueue {
    pub store: Arc<RwLock<TaskStore>>,
    processor_handle: JoinHandle<()>,
    event_tx: mpsc::UnboundedSender<TaskExecuteInnerEvent>,
    semaphore: Arc<Semaphore>,
}

impl TaskQueue {
    pub fn new(max_concurrent: usize, event_tx: broadcast::Sender<TransferEvent>) -> Self {
        let (
            task_event_tx,
            task_event_rx,
        ) = mpsc::unbounded_channel();
        let store = Arc::new(RwLock::new(TaskStore::new()));

        let task_event_processor = TaskEventProcessor {
            task_event_rx,
            event_tx,
            store: store.clone(),
        };
        let processor_handle = tokio::spawn(task_event_processor.run());

        Self {
            store,
            processor_handle,
            event_tx: task_event_tx,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
        }
    }

    pub async fn try_execute_next(&mut self) {
        if self.semaphore.available_permits() == 0 {
            return;
        }

        let mut store_guard = self.store.write().await;
        if let Some(task_id) = store_guard.get_next_pending() {
            if let Some(task) = store_guard.get_task(&task_id) {
                let handle = self.execute(task).await;
                store_guard.add_running(task_id, handle);
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
        self.store.write().await.add_task(task);
    }
}
