use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, broadcast};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use crate::tus::progress_aggregator::ProgressAggregator;
use super::task::UploadTask;
use super::worker::UploadWorker;
use super::client::TusClient;
use super::errors::{Result, TusError};
use super::types::{
    ManagerCommand,
    UploadConfig,
    UploadEvent,
    UploadId,
    UploadState
};

struct TaskHandle {
    task: UploadTask,
    cancellation_token: Option<CancellationToken>,
    join_handle: Option<JoinHandle<Result<String>>>,
}

/// 上传管理器的工作线程
pub struct UploadManagerWorker {
    /// TUS 客户端的实现
    client: TusClient,

    /// 配置文件
    config: UploadConfig,

    /// 所有的任务
    tasks: HashMap<UploadId, TaskHandle>,

    /// 在队列中的任务
    queued_tasks: Vec<UploadId>,

    /// 进行中的任务
    active_uploads: HashSet<UploadId>,

    /// 事件触发通知
    event_tx: broadcast::Sender<UploadEvent>,

    /// 任务完成通知
    task_completion_rx: mpsc::UnboundedReceiver<UploadId>,
    task_completion_tx: mpsc::UnboundedSender<UploadId>,

    /// 进度聚合器
    progress_aggregator: Arc<ProgressAggregator>,

    /// 开始时间
    start_time: Instant,
}

impl UploadManagerWorker {
    pub(crate) async fn run(
        client: TusClient,
        config: UploadConfig,
        mut command_rx: mpsc::Receiver<ManagerCommand>,
        event_tx: broadcast::Sender<UploadEvent>,
        progress_aggregator: Arc<ProgressAggregator>,
    ) {
        let (task_completion_tx, task_completion_rx) = mpsc::unbounded_channel();
        let mut worker = Self {
            client,
            config,
            tasks: HashMap::new(),
            queued_tasks: Vec::new(),
            active_uploads: HashSet::new(),
            event_tx,
            task_completion_tx,
            task_completion_rx,
            progress_aggregator,
            start_time: Instant::now(),
        };

        // 恢复之前的状态
        if worker.config.auto_save_state {
            if let Err(err) = worker.restore_state().await {
                eprintln!("Failed to restore state: {}", err);
            }
        }

        // 主事件循环, 循环等待命令
        loop {
            tokio::select! {
                Some(command) = command_rx.recv() => {
                    worker.handle_command(command).await;
                }
                Some(upload_id) = worker.task_completion_rx.recv() => {
                    worker.handle_task_completion(upload_id).await;
                }
                else => break
            }

            worker.process_queue().await;

            // 保存状态
            if worker.config.auto_save_state {
                if let Err(err) = worker.save_state().await {
                    eprintln!("Failed to save state: {}", err);
                }
            }
        }
    }

    async fn handle_command(&mut self, command: ManagerCommand) {
        match command {
            ManagerCommand::AddUpload { file_path, metadata, reply } => {
                let result = self.add_upload(file_path, metadata).await;
                let _ = reply.send(result);
            }
            ManagerCommand::BatchAdd { files, reply } => {
                let mut upload_ids = Vec::new();
                for (file_path, metadata) in files {
                    match self.add_upload(file_path, metadata).await {
                        Ok(upload_id) => upload_ids.push(upload_id),
                        Err(err) => {
                            let _ = reply.send(Err(err));
                            return;
                        }
                    }
                }
                let _ = reply.send(Ok(upload_ids));
            }
            ManagerCommand::PauseUpload { upload_id, reply } => {
                let result = self.pause_upload(upload_id).await;
                let _ = reply.send(result);
            }
            ManagerCommand::PauseAll { reply } => {
                for id in self.queued_tasks.clone() {
                    let _ = self.pause_upload(id).await;
                }
                for id in self.active_uploads.clone() {
                    let _ = self.pause_upload(id).await;
                }
                let _ = reply.send(Ok(()));
            }
            ManagerCommand::ResumeUpload { upload_id, reply } => {
                let result = self.resume_upload(upload_id).await;
                let _ = reply.send(result);
            }
            ManagerCommand::ResumeAll { reply } => {
                let paused_tasks: Vec<_> = self.tasks
                    .iter()
                    .filter(|(_, handle)| handle.task.state == UploadState::Paused)
                    .map(|(id, _)| id.clone())
                    .collect();

                for id in paused_tasks {
                    let _ = self.resume_upload(id).await;
                }
                let _ = reply.send(Ok(()));
            }
            ManagerCommand::CancelUpload { upload_id, reply } => {
                let result = self.cancel_upload(upload_id).await;
                let _ = reply.send(result);
            }
            ManagerCommand::CancelAll { reply } => {
                let all_ids: Vec<_> = self.tasks.keys().cloned().collect();
                for id in all_ids {
                    let _ = self.cancel_upload(id).await;
                }
                let _ = reply.send(Ok(()));
            }
            ManagerCommand::GetTask { upload_id, reply } => {
                let task = self.tasks
                    .get(&upload_id)
                    .map(|handle| handle.task.clone());
                let _ = reply.send(task);
            }
            ManagerCommand::GetAllTasks { reply } => {
                let tasks: Vec<_> = self.tasks
                    .values()
                    .map(|handle| handle.task.clone())
                    .collect();
                let _ = reply.send(tasks);
            }
            ManagerCommand::GetStats { reply } => {
                let stats = self.progress_aggregator.get_current_stats();
                let _ = reply.send(stats);
            }
            ManagerCommand::Clean { reply } => {
                let mut cleaned = 0;
                self.tasks.retain(|_, task_handle| {
                    let should_keep = task_handle.task.state != UploadState::Completed
                        && task_handle.task.state != UploadState::Failed;
                    
                    if !should_keep {
                        cleaned += 1;
                    }
                    
                    should_keep
                });
                
                let _ = reply.send(cleaned);
            }
            ManagerCommand::SetProgressInterval { interval, reply } => {
                // 暂未实现动态更新间隔
                let _ = reply.send(Ok(()));
            }
        }
    }

    async fn process_queue(&mut self) {
        while self.active_uploads.len() < self.config.concurrent && !self.queued_tasks.is_empty() {
            let upload_id = self.queued_tasks.remove(0);

            if let Some(handle) = self.tasks.get_mut(&upload_id) {
                if handle.task.state == UploadState::Queued {
                    self.start_upload(upload_id).await;
                }
            }
        }
    }

    async fn start_upload(&mut self, upload_id: UploadId) {
        let handle = match self.tasks.get_mut(&upload_id) {
            Some(h) => h,
            None => return
        };

        // 更新状态为准备中
        // let _ = self.event_tx.send(UploadEvent::StateChanged {
        //     upload_id,
        //     old_state: handle.task.state,
        //     new_state: UploadState::Preparing
        // });
        self.emit_state_change(upload_id, handle.task.state, UploadState::Preparing);
        handle.task.state = UploadState::Preparing;

        // 创建上传URL（如果还没有）
        if handle.task.upload_url.is_none() {
            match self.client.create_upload(handle.task.file_size, handle.task.metadata.clone()).await {
                Ok(upload_url) => {
                    handle.task.upload_url = Some(upload_url);
                }
                Err(err) => {
                    handle.task.state = UploadState::Failed;
                    handle.task.error = Some(err.to_string());
                    self.emit_state_change(upload_id, UploadState::Preparing, UploadState::Failed);
                    let _ = self.event_tx.send(UploadEvent::Failed { upload_id, error: err.to_string() });
                    return;
                }
            }
        }

        // 创建 CancellationToken
        let cancellation_token = CancellationToken::new();
        handle.cancellation_token = Some(cancellation_token.clone());

        // 创建 Worker
        let worker = UploadWorker {
            cancellation_token,
            client: self.client.clone(),
            config: self.config.clone(),
            progress_aggregator: if self.config.batch_progress {
                Some(self.progress_aggregator.clone())
            } else {
                None
            },
        };

        let task = handle.task.clone();
        let completion_tx = self.task_completion_tx.clone();

        // 启动上传任务
        let join_handle = tokio::spawn(async move {
            let result = worker.run(task).await;
            let _ = completion_tx.send(upload_id);
            result
        });

        handle.join_handle = Some(join_handle);
        handle.task.state = UploadState::Uploading;
        handle.task.started_at = Some(chrono::Utc::now());

        self.queued_tasks.retain(|id| *id != upload_id);
        self.active_uploads.insert(upload_id);

        self.emit_state_change(upload_id, UploadState::Preparing, UploadState::Uploading);
    }

    async fn add_upload(&mut self, file_path: PathBuf, metadata: Option<HashMap<String, String>>) -> Result<UploadId> {
        let file_metadata = tokio::fs::metadata(&file_path).await?;
        if !file_metadata.is_file() {
            return Err(TusError::ParamError("Not a file".to_string()));
        }

        let upload_id = UploadId::new();
        let task = UploadTask {
            id: upload_id,
            file_path,
            file_size: file_metadata.len(),
            upload_url: None,
            state: UploadState::Queued,
            bytes_uploaded: 0,
            created_at: chrono::Utc::now(),
            started_at: None,
            completed_at: None,
            error: None,
            metadata,
        };

        self.tasks.insert(upload_id, TaskHandle {
            task,
            cancellation_token: None,
            join_handle: None
        });

        self.queued_tasks.push(upload_id);
        self.emit_state_change(upload_id, UploadState::Queued, UploadState::Queued);

        Ok(upload_id)
    }

    async fn pause_upload(&mut self, upload_id: UploadId) -> Result<()> {
        let handle = self.tasks.get_mut(&upload_id)
            .ok_or_else(|| TusError::ParamError("Task not found".to_string()))?;

        match handle.task.state {
            UploadState::Queued => {
                self.queued_tasks.retain(|id| *id != upload_id);
                handle.task.state = UploadState::Paused;
                self.emit_state_change(upload_id, UploadState::Queued, UploadState::Paused);
            }
            UploadState::Uploading => {
                if let Some(token) = &handle.cancellation_token {
                    token.cancel();
                }

                handle.task.state = UploadState::Paused;
                self.active_uploads.remove(&upload_id);
                self.emit_state_change(upload_id, UploadState::Uploading, UploadState::Paused);
            }
            _ => {}
        }

        Ok(())
    }

    async fn resume_upload(&mut self, upload_id: UploadId) -> Result<()> {
        let handle = self.tasks.get_mut(&upload_id)
            .ok_or_else(|| TusError::ParamError("Task not found".to_string()))?;

        match handle.task.state {
            UploadState::Paused => {
                handle.task.state = UploadState::Queued;
                self.queued_tasks.push(upload_id);
                self.emit_state_change(upload_id, UploadState::Paused, UploadState::Queued);
            }
            _ => {}
        }

        Ok(())
    }

    async fn cancel_upload(&mut self, upload_id: UploadId) -> Result<()> {
        let handle = self.tasks.get_mut(&upload_id)
            .ok_or_else(|| TusError::ParamError("Task not found".to_string()))?;

        // 取消正在进行的上传
        if let Some(token) = &handle.cancellation_token {
            self.queued_tasks.retain(|id| *id != upload_id);
            self.active_uploads.remove(&upload_id);
            token.cancel();
        }

        // 尝试在服务器端取消上传
        if let Some(upload_url) = &handle.task.upload_url {
            let _ = self.client.cancel_upload(upload_url).await;
        }

        let old_state = handle.task.state;
        self.tasks.remove(&upload_id);
        self.emit_state_change(upload_id, old_state, UploadState::Cancelled);

        Ok(())
    }

    async fn handle_task_completion(&mut self, upload_id: UploadId) {
        let handle = match self.tasks.get_mut(&upload_id) {
            Some(handle) => handle,
            None => return
        };

        if let Some(join_handle) = handle.join_handle.take() {
            let old_state = handle.task.state;
            match join_handle.await {
                Ok(Ok(upload_url)) => {
                    if old_state == UploadState::Uploading {
                        handle.task.state = UploadState::Completed;
                        handle.task.completed_at = Some(chrono::Utc::now());
                        handle.task.bytes_uploaded = handle.task.file_size;

                        self.active_uploads.remove(&upload_id);
                        self.emit_state_change(upload_id, old_state, UploadState::Completed);

                        let _ = self.event_tx.send(UploadEvent::Completed { upload_id, upload_url });

                        todo!("check_all_completed")
                    }
                }
                Ok(Err(err)) => {
                    // 由 Cancellation Token 触发
                    if matches!(err, TusError::Cancelled){
                        return;
                    }
                    
                    let err_message = err.to_string();
                    handle.task.state = UploadState::Failed;
                    handle.task.error = Some(err_message.clone());

                    self.active_uploads.remove(&upload_id);
                    self.emit_state_change(upload_id, old_state, UploadState::Failed);
                    let _ = self.event_tx.send(UploadEvent::Failed {
                        upload_id,
                        error: err_message
                    });
                }
                Err(err) => {
                    handle.task.state = UploadState::Failed;
                    handle.task.error = Some(format!("Task panicked: {}", err));
                    self.active_uploads.remove(&upload_id);
                    self.emit_state_change(upload_id, old_state, UploadState::Failed);
                }
            }
        }
    }

    fn emit_state_change(&self, upload_id: UploadId, old_state: UploadState, new_state: UploadState) {
        let _ = self.event_tx.send(UploadEvent::StateChanged {
            upload_id,
            old_state,
            new_state
        });
    }

    /// Save tasks state
    async fn save_state(&self) -> Result<()> {
        if let Some(state_file) = &self.config.state_file {
            let tasks: Vec<_> = self.tasks.values().map(|h| &h.task).collect();
            let data = serde_json::to_string_pretty(&tasks)?;
            tokio::fs::write(state_file, data).await?;
        }

        Ok(())
    }

    /// Restore tasks from the machine
    async fn restore_state(&mut self) -> Result<()> {
        if let Some(path) = &self.config.state_file {
            if path.exists() {
                // 读取配置
                let data = tokio::fs::read_to_string(path).await?;
                let tasks: Vec<UploadTask> = serde_json::from_str(&data)
                    .unwrap_or_default();

                for task in tasks {
                    // 恢复未完成的任务
                    match task.state {
                        UploadState::Queued | UploadState::Uploading | UploadState::Paused => {
                            let upload_id = task.id;
                            let mut restored_task = task;

                            // 将上传中的任务重置为队列状态
                            if restored_task.state == UploadState::Uploading {
                                restored_task.state = UploadState::Queued;
                            }

                            self.tasks.insert(upload_id, TaskHandle {
                                task: restored_task,
                                cancellation_token: None,
                                join_handle: None,
                            });

                            if self.tasks[&upload_id].task.state == UploadState::Queued {
                                self.queued_tasks.push(upload_id);
                            }
                        }
                        _ => {
                            // 保留已完成/失败/取消的任务记录
                            self.tasks.insert(task.id, TaskHandle {
                                task,
                                cancellation_token: None,
                                join_handle: None,
                            });
                        }
                    }
                }
            }
        }
        
        Ok(())
    }
}
