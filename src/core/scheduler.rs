use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;
use futures_util::StreamExt;
use tokio::sync::{broadcast, mpsc, oneshot, RwLock, Semaphore};
use tokio_util::sync::CancellationToken;
use crate::progress::ProgressAggregator;
use crate::{TransferContext, TransferEvent, TransferId, TransferProtocol, TransferState, TransferTask};
use super::{
    Result,
    TransferError,
    TransferProtocolBuilder,
};

pub enum ManagerCommand {
    AddTask {
        builder: Box<dyn TransferProtocolBuilder>,
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

        Self {
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
        self.event_tx.closed().await;
        self.command_tx.closed().await;
    }

    pub async fn add_task(&self, builder: Box<dyn TransferProtocolBuilder>) -> Result<TransferId> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(ManagerCommand::AddTask { builder, reply: reply_tx })
            .map_err(|_| TransferError::ManagerShutdown)?;

        let id = reply_rx
            .await
            .map_err(|_| TransferError::ManagerShutdown)??;

        Ok(id)
    }

    pub async fn cancel_task(&self, id: TransferId) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(ManagerCommand::CancelTask { id, reply: reply_tx })
            .map_err(|_| TransferError::ManagerShutdown)?;
        
        let _ = reply_rx.await.map_err(|_| TransferError::ManagerShutdown)?;

        Ok(())
    }

    pub async fn pause_task(&self, id: TransferId) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(ManagerCommand::PauseTask { id, reply: reply_tx })
            .map_err(|_| TransferError::ManagerShutdown)?;

        let _ = reply_rx.await.map_err(|_| TransferError::ManagerShutdown)?;

        Ok(())
    }

    pub async fn resume_task(&self, _id: TransferId) -> Result<()> {
        let (_reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(ManagerCommand::ResumeTask)
            .map_err(|_| TransferError::ManagerShutdown)?;

        let _ = reply_rx
            .await
            .map_err(|_| TransferError::ManagerShutdown)?;

        Ok(())
    }
}

#[derive(Default)]
struct TaskManager {
    tasks: HashMap<TransferId, TransferTask>,
    pending_tasks: VecDeque<TransferId>,
    running_tasks: HashMap<TransferId, CancellationToken>,
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

    /// Semaphore, concurrent
    semaphore: Arc<Semaphore>,

    /// Cancellation token
    system_token: CancellationToken,

    /// Progress
    aggregator: ProgressAggregator,
}

impl CommandHandler {
    fn new(
        cancellation_token: CancellationToken,
        command_rx: mpsc::UnboundedReceiver<ManagerCommand>,
        event_tx: broadcast::Sender<TransferEvent>,
    ) -> Self {
        let (task_event_tx, task_event_rx) = mpsc::unbounded_channel();
        let manager = Default::default();

        let aggregator = ProgressAggregator {
            trackers: Default::default(),
            event_tx: event_tx.clone(),
            cancellation_token: cancellation_token.clone(),
        }.enable_report();

        Self {
            command_rx,
            event_tx,
            system_token: cancellation_token,
            task_event_tx,
            task_event_rx,
            manager,
            aggregator,
            semaphore: Arc::new(Semaphore::new(1)),
        }
    }

    async fn run(mut self) {
        loop {
            tokio::select! {
                _ = self.system_token.cancelled() => break,
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
            ManagerCommand::AddTask { reply, builder } => {
                let result = self.add_task(builder).await;
                let _ = reply.send(result);
                self.schedule_task().await;
            }
            ManagerCommand::PauseTask { .. } => {}
            ManagerCommand::ResumeTask => {}
            ManagerCommand::CancelTask { id, reply } => {
                let _ = self.cancel_task(id).await;
                self.aggregator.unregister_task(&id).await;
                let _ = reply.send(());
                let _ = self.event_tx.send(TransferEvent::Cancelled { id });
                self.schedule_task().await;
            }
        }
    }

    async fn handle_event(&self, event: TaskExecuteEvent) {
        match event {
            TaskExecuteEvent::Execute { id } => {
                let _ = self.event_tx.send(TransferEvent::Started { id });
                let mut manager = self.manager.write().await;
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
                    self.schedule_task().await;
                }
            }
            TaskExecuteEvent::Failed { .. } => {}
        }
    }

    async fn add_task(&mut self, builder: Box<dyn TransferProtocolBuilder>) -> Result<TransferId> {
        let id = TransferId::new();
        let task = TransferTask {
            id: id.clone(),
            builder,
            state: TransferState::Queued,
            created_at: Instant::now(),
            started_at: None,
            completed_at: None,
        };

        let mut manager = self.manager.write().await;
        manager.tasks.insert(id.clone(), task);
        manager.pending_tasks.push_front(id);

        Ok(id)
    }

    async fn schedule_task(&self) {
        if self.semaphore.available_permits() == 0 {
            println!("meiyou");
            return;
        }

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
            // progress
            let (bytes_tx, bytes_rx) = mpsc::unbounded_channel();
            self.aggregator.registry_task(task.id, bytes_rx).await;

            // 协议
            let mut context = task.builder.build_context();
            let protocol = task.builder.build_protocol();

            // inner vars
            let transfer_id = next_id.clone();
            let task_event_tx = self.task_event_tx.clone();
            
            // token
            let cancellation_token = CancellationToken::new();
            let task_token = cancellation_token.clone();

            tokio::spawn(async move {
                let result = protocol.initialize(&mut context).await;
                let future = CommandHandler::execute_task(protocol, bytes_tx, context, );
                let _ = task_event_tx.send(TaskExecuteEvent::Execute { id: transfer_id });
                tokio::select! {
                    _ = task_token.cancelled() => {
                        println!("Cancelled task");
                        drop(permit);
                    },
                    result = future => {
                        match result {
                            Ok(_) => {
                                let _ = task_event_tx.send(TaskExecuteEvent::Completed { id: transfer_id });
                            }
                            Err(err) => {
                                let _ = task_event_tx.send(TaskExecuteEvent::Failed {
                                    id: transfer_id,
                                    reason: err.to_string()
                                });
                            }
                        }
                    }
                }
            });

            // 插入任务
            manager.running_tasks.insert(next_id, cancellation_token);
        }
    }

    async fn execute_task(
        protocol: Box<dyn TransferProtocol>,
        bytes_tx: mpsc::UnboundedSender<u64>,
        mut context: TransferContext,
    ) -> Result<()> {
        protocol.initialize(&mut context).await?;
        protocol.execute(&context, Some(bytes_tx)).await?;
        Ok(())
    }

    async fn cancel_task(&self, id: TransferId) -> Result<()> {
        let mut manager = self.manager.write().await;
        if let Some(task) = manager.tasks.remove(&id) {
            match task.state {
                TransferState::Queued => {
                    manager.pending_tasks.retain(|x| *x != id);
                }
                TransferState::Running => {
                    println!("Cancel task: {:?}", id);
                    if let Some(token) = manager.running_tasks.remove(&id) {
                        token.cancelled_owned().await;
                    }
                }
                _ => {}
                // TransferState::Paused => {}
                // TransferState::Completed => {}
                // TransferState::Failed => {}
                // TransferState::Cancelled => {}
            }
        }

        Ok(())
    }
}
