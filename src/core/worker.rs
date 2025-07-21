use std::pin::Pin;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;
use tokio::sync::mpsc;
use crate::core::{TransferChunk, TransferError, TransferHook};
use super::errors::Result;
use super::traits::TransferProcessor;
use super::progress::ProgressTracker;
use super::task::{TaskControl, TransferTask};
use super::types::{TransferConfig, TransferState};

pub struct TransferWorker {
    id: usize,
    config: TransferConfig,
    progress_tracker: Option<Arc<dyn ProgressTracker>>,
    hooks: Vec<Arc<dyn TransferHook>>,
    cancellation_token: CancellationToken,
}

impl TransferWorker {
    pub fn new(id: usize, config: TransferConfig, cancellation_token: CancellationToken) -> Self {
        Self {
            id,
            config,
            cancellation_token,
            hooks: Vec::new(),
            progress_tracker: None,
        }
    }

    pub fn with_progress_tracker(mut self, tracker: Arc<dyn ProgressTracker>) -> Self {
        self.progress_tracker = Some(tracker);
        self
    }

    pub fn add_hook(mut self, hook: Arc<dyn TransferHook>) -> Self {
        self.hooks.push(hook);
        self
    }

    pub async fn execute(
        self,
        task: TransferTask,
        mut processor: Box<dyn TransferProcessor>,
        control_rx: mpsc::Receiver<TaskControl>,
    ) -> Result<()> {
        log::debug!("Worker {} starting task {}", self.id, task.id);

        let context = task.context.read().await.clone();
        for hook in &self.hooks {
            hook.before_transfer(&context).await?;
        }

        let result = self.execute_with_retry(&task, &mut processor, control_rx).await;

        match result {
            Ok(_) => {
                for hook in &self.hooks {
                    hook.after_transfer(&context).await?;
                }
            }
            Err(err) => {
                for hook in &self.hooks {
                    hook.on_error(&context, &err).await?;
                }
            }
        }

        processor.finalize().await?;

        Ok(())
    }

    async fn execute_with_retry(
        &self,
        task: &TransferTask,
        processor: &mut Box<dyn TransferProcessor>,
        mut control_tx: mpsc::Receiver<TaskControl>
    ) -> Result<()> {
        let retry_config = &self.config.retry_config;
        let mut retry_count = 0;
        let mut delay = retry_config.initial_delay;

        loop {
            match self.execute_one(task, processor, &mut control_tx).await {
                Ok(_) => return Ok(()),
                Err(err) => {
                    if !err.is_retryable() || retry_count >= retry_config.max_retries {
                        return Err(err);
                    }

                    retry_count += 1;
                    task.increment_retry_count().await;

                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {},
                        _ = self.cancellation_token.cancelled() => {
                            return Err(TransferError::Cancelled);
                        }
                    }

                    delay = std::cmp::max(
                        delay.mul_f64(retry_config.multiplier),
                        retry_config.max_delay
                    );
                }
            }
        }
    }

    async fn execute_one(
        &self,
        task: &TransferTask,
        processor: &mut Box<dyn TransferProcessor>,
        control_rx: &mut mpsc::Receiver<TaskControl>
    ) -> Result<()> {
        task.set_state(TransferState::Preparing).await?;

        let mut context = task.context.write().await;
        task.protocol.initialize(&mut context).await?;
        *task.context.write().await = context.clone();

        let process = task.protocol.get_progress(&context).await?;
        task.update_progress(process).await;

        if let Some(tracker) = &self.progress_tracker {
            tracker.register_task(
                task.id,
                context.metadata.as_ref().and_then(|m| m.total_size).unwrap_or(0),
            ).await;
        }

        task.set_state(TransferState::Transferring).await?;

        let stream = task.protocol.create_stream(&context).await?;
        let result = self.process_stream(
            task,
            processor,
            stream,
            control_rx
        ).await;

        if let Some(tracker) = &self.progress_tracker {
            tracker.unregister_task(task.id).await;
        }

        match &result {
            Ok(_) => {
                task.protocol.finalize(&mut *task.context.write().await).await?;
                task.set_state(TransferState::Completed).await?;
            }
            Err(err) => {
                match err {
                    TransferError::Cancelled => {
                        task.set_state(TransferState::Cancelled).await?;
                        task.protocol.cancel(&context).await?;
                    }
                    _ => {
                        task.set_state(TransferState::Failed).await?;
                        task.set_error(Some(err.to_string())).await;
                    }
                }
            }
        }

        result
    }

    async fn process_stream(
        &self,
        task: &TransferTask,
        processor: &mut Box<dyn TransferProcessor>,
        mut stream: Pin<Box<dyn futures::Stream<Item = Result<TransferChunk>> + Send>>,
        control_rx: &mut mpsc::Receiver<TaskControl>
    ) -> Result<()> {
        let mut paused = false;
        let mut bytes_transferred = task.stats.read().await.bytes_transferred;

        loop {
            match control_rx.try_recv() {
                Ok(TaskControl::Pause) => {
                    if !paused {
                        task.set_state(TransferState::Paused).await?;
                        paused = true;
                    }
                }
                Ok(TaskControl::Resume) => {
                    if paused {
                        task.set_state(TransferState::Transferring).await?;
                        paused = false;
                    }
                }
                Ok(TaskControl::Cancel) => {
                    return Err(TransferError::Cancelled);
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err(TransferError::Internal("Control channel disconnected".to_string()));
                }
            }

            if paused {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }

            tokio::select! {
                chunk_result = stream.next() => {
                    match chunk_result {
                        Some(Ok(chunk)) => {
                            let context = task.context.read().await.clone();
                            for hook in &self.hooks {
                                hook.before_chunk(&context, &chunk).await?;
                            }

                            let chunk_size = chunk.data.len() as u64;
                            let is_final = chunk.is_final;
                            processor.process_chunk(chunk).await?;

                            bytes_transferred += chunk_size;
                            task.update_progress(bytes_transferred).await;

                            if let Some(tracker) = &self.progress_tracker {
                                tracker.update_progress(task.id, bytes_transferred).await;
                            }

                            if is_final {
                                break;
                            }
                        }
                        Some(Err(e)) => {
                            return Err(e);
                        }
                        None => {
                            break;
                        }
                    }
                }
                _ = self.cancellation_token.cancelled() => {
                    return Err(TransferError::Cancelled);
                }
            }
        }

        Ok(())
    }
}

pub struct WorkerPool {
    workers: Vec<WorkerHandle>,
    config: TransferConfig,
    cancellation_token: CancellationToken,
}

impl WorkerPool {
    pub fn new(size: usize, config: TransferConfig) -> Self {
        let cancellation_token = CancellationToken::new();

        Self {
            workers: Vec::with_capacity(size),
            config,
            cancellation_token
        }
    }

    pub fn start(
        mut self,
        task_rx: mpsc::Receiver<WorkerTask>,
        progress_tracker: Option<Arc<dyn ProgressTracker>>
    ) -> Self {
        let size = self.workers.len();

        for id in 0..size {
            let worker = TransferWorker::new(
                id,
                self.config.clone(),
                self.cancellation_token.child_token(),
            );

            if let Some(ref tracker) = progress_tracker {
                worker.with_progress_tracker(tracker.clone());
            }

            // let handle = WorkerHandle::spawn(id, worker, task_rx);
            // self.workers.push(handle);
        }

        self
    }

    pub async fn shutdown(self) {
        self.cancellation_token.cancel();

        for handle in self.workers {
            let _ = handle.join().await;
        }
    }
}

struct WorkerHandle {
    id: usize,
    handle: tokio::task::JoinHandle<()>,
}

impl WorkerHandle {
    fn spawn(id: usize, worker: TransferWorker, mut task_rx: mpsc::Receiver<WorkerTask>) -> Self {
        let handle = tokio::spawn(async move {
            while let Some(task) = task_rx.recv().await {
                // let result = worker.execute(
                //     task.task,
                //     task.processor,
                //     task.control_rx,
                // ).await;
                //
                // let _ = task.completion_tx.send(result);
            }
        });

        Self { id, handle }
    }

    async fn join(self) -> Result<()> {
        self.handle
            .await
            .map_err(|e| TransferError::Internal(format!("Worker {} panicked: {}", self.id, e)))
    }
}

pub(crate) struct WorkerTask {
    pub task: TransferTask,
    pub processor: Box<dyn TransferProcessor>,
    pub control_rx: mpsc::Receiver<TaskControl>,
    pub completion_tx: tokio::sync::oneshot::Sender<Result<()>>,
}
