use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;
use futures_util::StreamExt;
use tokio::sync::{broadcast, mpsc, oneshot, OwnedSemaphorePermit, RwLock, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use crate::progress::ProgressAggregator;
use crate::{TransferEvent, TransferId, TransferState, TransferTask};
use super::{
    Result,
    TransferError,
    TransferProtocolBuilder,
};

pub enum ManagerCommand {
    AddTask {
        builder: Box<dyn TransferProtocolBuilder>,
        bytes_tx: mpsc::UnboundedSender<u64>,
        reply: oneshot::Sender<Result<TransferId>>,
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

pub struct Tunnel {
    command_tx: mpsc::UnboundedSender<ManagerCommand>,
    event_tx: broadcast::Sender<TransferEvent>,
    aggregator: ProgressAggregator,
    cancellation_token: CancellationToken,
}

impl Tunnel {
    pub fn new() -> Self {
        let cancellation_token = CancellationToken::new();
        let (event_tx, _) = broadcast::channel(128);
        let (command_tx, command_rx) = mpsc::unbounded_channel();

        let command_handler = CommandHandler::new(
            cancellation_token.clone(),
            command_rx,
            event_tx.clone(),
        );
        tokio::spawn(command_handler.run());

        let aggregator = ProgressAggregator {
            trackers: Default::default(),
            event_tx: event_tx.clone(),
            cancellation_token: cancellation_token.clone(),
        }.enable_report();

        Self {
            aggregator,
            command_tx,
            event_tx,
            cancellation_token
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TransferEvent> {
        self.event_tx.subscribe()
    }

    pub async fn shutdown(&mut self) {
        self.cancellation_token.cancel();
        self.aggregator.clear().await;
        self.event_tx.closed().await;
        self.command_tx.closed().await;
    }

    pub async fn add_task(&self, builder: Box<dyn TransferProtocolBuilder>) -> Result<TransferId> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let (bytes_tx, bytes_rx) = mpsc::unbounded_channel();

        self.command_tx
            .send(ManagerCommand::AddTask { builder, reply: reply_tx, bytes_tx })
            .map_err(|_| TransferError::ManagerShutdown)?;

        let id = reply_rx
            .await
            .map_err(|_| TransferError::ManagerShutdown)??;

        self.aggregator.registry_task(id.clone(), bytes_rx).await;

        Ok(id)
    }

    pub async fn cancel_task(&self, id: TransferId) -> Result<()> {
        self.aggregator.unregister_task(&id).await;

        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(ManagerCommand::CancelTask { id, reply: reply_tx })
            .map_err(|_| TransferError::ManagerShutdown)?;
        
        let _ = reply_rx.await.map_err(|_| TransferError::ManagerShutdown)?;

        Ok(())
    }

    pub async fn pause_task(&self, id: TransferId) -> Result<()> {
        self.aggregator.unregister_task(&id).await;
        let (reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(ManagerCommand::PauseTask { id, reply: reply_tx })
            .map_err(|_| TransferError::ManagerShutdown)?;

        let _ = reply_rx.await.map_err(|_| TransferError::ManagerShutdown)?;

        Ok(())
    }

    pub async fn resume_task(&self, id: TransferId) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let (bytes_tx, bytes_rx) = mpsc::unbounded_channel();

        self.command_tx
            .send(ManagerCommand::ResumeTask)
            .map_err(|_| TransferError::ManagerShutdown)?;

        let _ = reply_rx
            .await
            .map_err(|_| TransferError::ManagerShutdown)?;

        self.aggregator.registry_task(id.clone(), bytes_rx).await;

        Ok(())
    }
}

#[derive(Default)]
struct TaskManager {
    tasks: HashMap<TransferId, TransferTask>,
    pending_tasks: VecDeque<TransferId>,
    running_tasks: HashMap<TransferId, JoinHandle<()>>,
}

enum ManagerWorkerEvent {
    Execute {
        id: TransferId,
        permit: OwnedSemaphorePermit,
        event_tx: broadcast::Sender<TransferEvent>,
    },
}

#[derive(Debug, Clone)]
enum TaskExecuteEvent {
    Execute { id: TransferId },
    Completed { id: TransferId },
    Failed { id: TransferId, reason: String },
}

struct CommandHandler {
    /// Task store
    manager: Arc<RwLock<TaskManager>>,

    /// ManagerCommand
    command_rx: mpsc::UnboundedReceiver<ManagerCommand>,

    /// Transfer event sender
    event_tx: broadcast::Sender<TransferEvent>,

    /// Inner event sender
    task_event_tx: mpsc::UnboundedSender<TaskExecuteEvent>,

    /// Inner event receiver
    task_event_rx: mpsc::UnboundedReceiver<TaskExecuteEvent>,

    /// Handle task
    worker_event_tx: mpsc::UnboundedSender<ManagerWorkerEvent>,

    /// Semaphore, concurrent
    semaphore: Arc<Semaphore>,

    /// Cancellation token
    cancellation_token: CancellationToken,
}

impl CommandHandler {
    fn new(
        cancellation_token: CancellationToken,
        command_rx: mpsc::UnboundedReceiver<ManagerCommand>,
        event_tx: broadcast::Sender<TransferEvent>,
    ) -> Self {
        let (worker_event_tx, worker_event_rx) = mpsc::unbounded_channel();
        let (task_event_tx, task_event_rx) = mpsc::unbounded_channel();
        let manager: Arc<RwLock<TaskManager>> = Default::default();

        Self {
            command_rx,
            event_tx,
            worker_event_tx,
            cancellation_token,
            task_event_tx,
            task_event_rx,
            manager,
            semaphore: Arc::new(Semaphore::new(3)),
        }
    }

    async fn run(mut self) {
        loop {
            tokio::select! {
                _ = self.cancellation_token.cancelled() => break,
                Some(command) = self.command_rx.recv() => {
                    self.handle_command(command).await;
                }
                Some(event) = self.task_event_rx.recv() => {
                    self.handle_event(event).await;
                }
            }
        }
    }

    async fn handle_command(&mut self, command: ManagerCommand) {
        match command {
            ManagerCommand::AddTask { reply, builder, bytes_tx, .. } => {
                let result = self.add_task(builder, bytes_tx).await;
                let _ = reply.send(result);
            }
            ManagerCommand::PauseTask { .. } => {}
            ManagerCommand::ResumeTask => {}
            ManagerCommand::CancelTask { .. } => {}
        }
    }

    async fn handle_event(&self, event: TaskExecuteEvent) {
        match event {
            TaskExecuteEvent::Execute { id } => {
                let _ = self.event_tx.send(TransferEvent::Started { id });
                let mut manager = self.manager.write().await;
                manager.pending_tasks.retain(|x| *x != id);
                if let Some(task) = manager.tasks.get_mut(&id) {
                    task.state = TransferState::Running;
                    task.started_at = Some(Instant::now());
                }
            }
            TaskExecuteEvent::Completed { id } => {
                let _ = self.event_tx.send(TransferEvent::Success { id });
                let mut manager = self.manager.write().await;
                manager.running_tasks.remove(&id);
                if let Some(task) = manager.tasks.get_mut(&id) {
                    task.state = TransferState::Completed;
                    task.completed_at = Some(Instant::now());
                }

                drop(manager);

                if self.semaphore.available_permits() != 0 {
                    self.try_execute_next().await;
                }
            }
            TaskExecuteEvent::Failed { .. } => {}
        }
    }

    async fn add_task(
        &mut self,
        builder: Box<dyn TransferProtocolBuilder>,
        bytes_tx: mpsc::UnboundedSender<u64>
    ) -> Result<TransferId> {
        let id = TransferId::new();
        let task = TransferTask {
            id: id.clone(),
            state: TransferState::Queued,
            builder,
            bytes_tx,
            created_at: Instant::now(),
            started_at: None,
            completed_at: None,
        };

        let mut manager = self.manager.write().await;
        manager.tasks.insert(id.clone(), task);
        manager.pending_tasks.push_front(id);
        drop(manager);

        if self.semaphore.available_permits() != 0 {
            self.try_execute_next().await;
        }

        Ok(id)
    }

    async fn try_execute_next(&self) {
        let mut manager = self.manager.write().await;
        let next_id = match manager.pending_tasks.pop_back() {
            Some(id) => id,
            None => return,
        };

        let permit = self.semaphore
            .clone()
            .acquire_owned()
            .await
            .unwrap();

        if let Some(task) = manager.tasks.get_mut(&next_id) {
            let task_event_tx = self.task_event_tx.clone();
            let progress_tx = task.bytes_tx.clone();
            
            let mut context = task.builder.build_context();
            let protocol = task.builder.build_protocol();

            let transfer_id = next_id.clone();
            let handle = tokio::spawn(async move {
                // initialize
                if let Err(error) = protocol.initialize(&mut context).await {
                    let _ = task_event_tx.send(TaskExecuteEvent::Failed {
                        id: transfer_id.clone(),
                        reason: error.to_string()
                    });
                };
                
                // execute
                match protocol.execute(&context, progress_tx).await {
                    Ok(_) => {
                        let _ = task_event_tx.send(TaskExecuteEvent::Completed { id: transfer_id });
                    },
                    Err(error) => {
                        let _ = task_event_tx.send(TaskExecuteEvent::Failed {
                            id: transfer_id.clone(),
                            reason: error.to_string()
                        });
                    }
                }
                
                drop(permit);
            });

            manager.running_tasks.insert(next_id, handle);
        }
    }
}
