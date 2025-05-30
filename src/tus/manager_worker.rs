use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tokio::sync::{mpsc, broadcast};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use super::task::UploadTask;
use super::worker::UploadWorker;
use super::client::TusClient;
use super::errors::{Result, TusError};
use super::types::{ManagerCommand, UploadEvent, UploadId, UploadState};

struct TaskHandle {
    task: UploadTask,
    cancellation_token: Option<CancellationToken>,
    join_handle: Option<JoinHandle<Result<String>>>,
}

pub struct UploadManagerWorker {
    client: TusClient,
    max_concurrent: usize,
    tasks: HashMap<UploadId, TaskHandle>,
    queued_tasks: Vec<UploadId>,
    active_uploads: HashSet<UploadId>,
    state_file: Option<PathBuf>,

    event_tx: broadcast::Sender<UploadEvent>,
    task_completion_rx: mpsc::UnboundedReceiver<UploadId>,
    task_completion_tx: mpsc::UnboundedSender<UploadId>,
}

impl UploadManagerWorker {
    pub(crate) async fn run(
        client: TusClient,
        max_concurrent: usize,
        state_file: Option<PathBuf>,
        mut command_rx: mpsc::Receiver<ManagerCommand>,
        event_tx: broadcast::Sender<UploadEvent>
    ) {
        let (task_completion_tx, task_completion_rx) = mpsc::unbounded_channel();
        let mut worker = Self {
            client,
            max_concurrent,
            tasks: HashMap::new(),
            queued_tasks: Vec::new(),
            active_uploads: HashSet::new(),
            state_file,
            event_tx,
            task_completion_tx,
            task_completion_rx,
        };

        // 恢复之前的状态
        if let Err(err) = worker.restore_state().await {
            eprintln!("Failed to restore state: {}", err);
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
            if let Err(err) = worker.save_state().await {
                eprintln!("Failed to save state: {}", err);
            }
        }
    }

    async fn handle_command(&mut self, command: ManagerCommand) {
        match command {
            ManagerCommand::AddUpload { file_path, metadata, reply } => {
                let result = self.add_upload(file_path, metadata).await;
                let _ = reply.send(result);
            }
            ManagerCommand::PauseUpload { upload_id, reply } => {
                let result = self.pause_upload(upload_id).await;
                let _ = reply.send(result);
            }
            ManagerCommand::ResumeUpload { upload_id, reply } => {
                let result = self.resume_upload(upload_id).await;
                let _ = reply.send(result);
            }
            ManagerCommand::CancelUpload { upload_id, reply } => {
                let result = self.cancel_upload(upload_id).await;
                let _ = reply.send(result);
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
            ManagerCommand::Clean { reply } => {
                self.tasks.retain(|_, task_handle| {
                    task_handle.task.state != UploadState::Completed && task_handle.task.state != UploadState::Failed
                });
                
                let _ = reply.send(Ok(()));
            }
        }
    }

    async fn process_queue(&mut self) {
        while self.active_uploads.len() < self.max_concurrent && !self.queued_tasks.is_empty() {
            let upload_id = self.queued_tasks.remove(0);

            if let Some(handle) = self.tasks.get_mut(&upload_id) {
                if handle.task.state == UploadState::Queued {
                    self.start_upload(upload_id).await;
                }
            }
        }
    }

    async fn start_upload(&mut self, upload_id: UploadId) {
        // 拿到任务的 handle 做上传
        let handle = match self.tasks.get_mut(&upload_id) {
            Some(h) => h,
            None => return
        };

        // 创建上传URL（如果还没有）
        if handle.task.upload_url.is_none() {
            match self.client.create_upload(handle.task.file_size, handle.task.metadata.clone()).await {
                Ok(upload_url) => {
                    handle.task.upload_url = Some(upload_url);
                }
                Err(err) => {
                    handle.task.state = UploadState::Failed;
                    handle.task.error = Some(err.to_string());
                    self.emit_state_change(upload_id, UploadState::Queued, UploadState::Failed);
                    let _ = self.event_tx.send(UploadEvent::Failed { upload_id, error: err.to_string() });
                    return;
                }
            }
        }

        // 创建 Cancellation_token
        let cancellation_token = CancellationToken::new();
        handle.cancellation_token = Some(cancellation_token.clone());

        let worker = UploadWorker {
            cancellation_token,
            client: self.client.clone()
        };

        let event_tx = self.event_tx.clone();
        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();

        let progress_forward = tokio::spawn({
            let event_tx = event_tx.clone();
            async move {
                while let Some(progress) = progress_rx.recv().await {
                    let _ = event_tx.send(UploadEvent::Progress(progress));
                }
            }
        });

        let task = handle.task.clone();
        let completion_tx = self.task_completion_tx.clone();
        let join_handle = tokio::spawn(async move {
            let result = worker.run(task, progress_tx).await;
            drop(progress_forward);
            let _ = completion_tx.send(upload_id);
            result
        });

        handle.join_handle = Some(join_handle);
        handle.task.state = UploadState::Uploading;
        handle.task.started_at = Some(chrono::Utc::now());

        self.queued_tasks.retain(|id| *id != upload_id);
        self.active_uploads.insert(upload_id);

        self.emit_state_change(upload_id, UploadState::Queued, UploadState::Uploading);
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
            token.cancel();
        }

        if let Some(upload_url) = &handle.task.upload_url {
            let res = self.client.cancel_upload(upload_url).await;
            if res.is_err() {
                eprintln!("{:?}", res);
            }
        }

        let old_state = handle.task.state;
        self.tasks.remove(&upload_id);
        self.queued_tasks.retain(|id| *id != upload_id);
        self.active_uploads.remove(&upload_id);
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

                        self.active_uploads.remove(&upload_id);
                        self.emit_state_change(upload_id, old_state, UploadState::Completed);

                        let _ = self.event_tx.send(UploadEvent::Completed { upload_id, upload_url });
                    }
                }
                Ok(Err(err)) => {
                    println!("Error: {:?}", err);
                    
                    // 由 Cancellation Token 触发
                    if matches!(err, TusError::Cancelled){
                        return;
                    }

                    handle.task.state = UploadState::Failed;
                    handle.task.error = Some(err.to_string());
                    self.emit_state_change(upload_id, old_state, UploadState::Failed);

                    let _ = self.event_tx.send(UploadEvent::Failed {
                        upload_id,
                        error: err.to_string()
                    });
                }
                Err(err) => {
                    handle.task.state = UploadState::Failed;
                    handle.task.error = Some(format!("Task panicked: {}", err));
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
        if let Some(state_file) = &self.state_file {
            let tasks: Vec<_> = self.tasks.values().map(|h| &h.task).collect();
            let data = serde_json::to_string_pretty(&tasks)?;
            tokio::fs::write(state_file, data).await?;
        }

        Ok(())
    }

    /// Restore tasks from the machine
    async fn restore_state(&mut self) -> Result<()> {
        if let Some(path) = &self.state_file {
            if path.exists() {
                // 读取配置
                let data = tokio::fs::read_to_string(path).await?;
                let tasks: Vec<UploadTask> = serde_json::from_str(&data)?;
                
                for task in tasks {
                    // 恢复未完成的任务
                    match task.state {
                        UploadState::Queued | UploadState::Uploading | UploadState::Paused => {
                            let upload_id = task.id;
                            self.tasks.insert(upload_id, TaskHandle {
                                task,
                                cancellation_token: None,
                                join_handle: None,
                            });

                            // 将暂停和上传中的任务加入队列
                            if matches!(self.tasks[&upload_id].task.state, UploadState::Uploading | UploadState::Paused) {
                                self.tasks.get_mut(&upload_id).unwrap().task.state = UploadState::Queued;
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
