use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex, RwLock};
use tokio::task::JoinHandle;
use chrono::Utc;

use super::errors::{Result, TransferError};
use super::traits::{Uploader, ProgressCallback, StorageAdapter};
use super::types::*;

/// 传输管理器
pub struct TransferManager {
    /// 上传器注册表
    uploaders: Arc<RwLock<HashMap<String, Arc<dyn Uploader + Send + Sync>>>>,
    
    /// 任务存储
    tasks: Arc<RwLock<HashMap<UploadId, UploadTask>>>,
    
    /// 活跃任务句柄
    active_tasks: Arc<Mutex<HashMap<UploadId, JoinHandle<()>>>>,
    
    /// 命令通道
    command_tx: mpsc::Sender<ManagerCommand>,
    
    /// 事件广播
    event_tx: broadcast::Sender<UploadEvent>,
    
    /// 配置
    config: Arc<TransferConfig>,
    
    /// 进度回调
    progress_callback: Option<Arc<dyn ProgressCallback + Send + Sync>>,
    
    /// 存储适配器
    storage_adapter: Option<Arc<dyn StorageAdapter + Send + Sync>>,
}

impl TransferManager {
    /// 创建新的传输管理器
    pub fn new(config: TransferConfig) -> (Self, JoinHandle<()>) {
        let (command_tx, command_rx) = mpsc::channel(config.queue_size);
        let (event_tx, _) = broadcast::channel(1024);
        
        let manager = Self {
            uploaders: Arc::new(RwLock::new(HashMap::new())),
            tasks: Arc::new(RwLock::new(HashMap::new())),
            active_tasks: Arc::new(Mutex::new(HashMap::new())),
            command_tx,
            event_tx: event_tx.clone(),
            config: Arc::new(config),
            progress_callback: None,
            storage_adapter: None,
        };
        
        let worker = manager.clone();
        let handle = tokio::spawn(async move {
            worker.run_worker(command_rx).await;
        });
        
        (manager, handle)
    }
    
    /// 注册上传器
    pub async fn register_uploader(&self, name: &str, uploader: Arc<dyn Uploader + Send + Sync>) -> Result<()> {
        let mut uploaders = self.uploaders.write().await;
        if uploaders.contains_key(name) {
            return Err(TransferError::already_exists(format!("Uploader '{}' already registered", name)));
        }
        uploaders.insert(name.to_string(), uploader);
        Ok(())
    }
    
    /// 设置进度回调
    pub fn set_progress_callback(&mut self, callback: Arc<dyn ProgressCallback + Send + Sync>) {
        self.progress_callback = Some(callback);
    }
    
    /// 设置存储适配器
    pub fn set_storage_adapter(&mut self, adapter: Arc<dyn StorageAdapter + Send + Sync>) {
        self.storage_adapter = Some(adapter);
    }
    
    /// 添加上传任务
    pub async fn add_upload(
        &self,
        file_path: PathBuf,
        strategy: UploadStrategy,
        metadata: HashMap<String, String>,
    ) -> Result<UploadId> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        
        self.command_tx
            .send(ManagerCommand::AddUpload {
                file_path,
                strategy,
                metadata,
                reply: reply_tx,
            })
            .await
            .map_err(|_| TransferError::internal("Manager shut down"))?;
        
        reply_rx.await.map_err(|_| TransferError::internal("Failed to receive reply"))?
    }
    
    /// 批量添加上传任务
    pub async fn add_uploads(
        &self,
        files: Vec<(PathBuf, Option<HashMap<String, String>>)>,
    ) -> Result<Vec<UploadId>> {
        let mut upload_ids = Vec::new();
        
        for (file_path, metadata) in files {
            let upload_id = self.add_upload(
                file_path,
                self.config.default_strategy.clone(),
                metadata.unwrap_or_default(),
            ).await?;
            upload_ids.push(upload_id);
        }
        
        Ok(upload_ids)
    }
    
    /// 暂停任务
    pub async fn pause(&self, upload_id: UploadId) -> Result<()> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        
        self.command_tx
            .send(ManagerCommand::Pause {
                upload_id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| TransferError::internal("Manager shut down"))?;
        
        reply_rx.await.map_err(|_| TransferError::internal("Failed to receive reply"))?
    }
    
    /// 恢复任务
    pub async fn resume(&self, upload_id: UploadId) -> Result<()> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        
        self.command_tx
            .send(ManagerCommand::Resume {
                upload_id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| TransferError::internal("Manager shut down"))?;
        
        reply_rx.await.map_err(|_| TransferError::internal("Failed to receive reply"))?
    }
    
    /// 取消任务
    pub async fn cancel(&self, upload_id: UploadId) -> Result<()> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        
        self.command_tx
            .send(ManagerCommand::Cancel {
                upload_id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| TransferError::internal("Manager shut down"))?;
        
        reply_rx.await.map_err(|_| TransferError::internal("Failed to receive reply"))?
    }
    
    /// 获取任务信息
    pub async fn get_task(&self, upload_id: UploadId) -> Option<UploadTask> {
        let tasks = self.tasks.read().await;
        tasks.get(&upload_id).cloned()
    }
    
    /// 获取所有任务
    pub async fn get_all_tasks(&self) -> Vec<UploadTask> {
        let tasks = self.tasks.read().await;
        tasks.values().cloned().collect()
    }
    
    /// 订阅事件
    pub fn subscribe_events(&self) -> broadcast::Receiver<UploadEvent> {
        self.event_tx.subscribe()
    }
    
    /// 关闭管理器
    pub async fn shutdown(&self) -> Result<()> {
        self.command_tx
            .send(ManagerCommand::Shutdown)
            .await
            .map_err(|_| TransferError::internal("Failed to send shutdown command"))?;
        Ok(())
    }
    
    /// 工作线程主循环
    async fn run_worker(&self, mut command_rx: mpsc::Receiver<ManagerCommand>) {
        while let Some(command) = command_rx.recv().await {
            match command {
                ManagerCommand::AddUpload {
                    file_path,
                    strategy,
                    metadata,
                    reply,
                } => {
                    let result = self.handle_add_upload(file_path, strategy, metadata).await;
                    let _ = reply.send(result);
                }
                ManagerCommand::Pause { upload_id, reply } => {
                    let result = self.handle_pause(upload_id).await;
                    let _ = reply.send(result);
                }
                ManagerCommand::Resume { upload_id, reply } => {
                    let result = self.handle_resume(upload_id).await;
                    let _ = reply.send(result);
                }
                ManagerCommand::Cancel { upload_id, reply } => {
                    let result = self.handle_cancel(upload_id).await;
                    let _ = reply.send(result);
                }
                ManagerCommand::GetTask { upload_id, reply } => {
                    let tasks = self.tasks.read().await;
                    let task = tasks.get(&upload_id).cloned();
                    let _ = reply.send(task);
                }
                ManagerCommand::GetAllTasks { reply } => {
                    let tasks = self.tasks.read().await;
                    let all_tasks = tasks.values().cloned().collect();
                    let _ = reply.send(all_tasks);
                }
                ManagerCommand::Shutdown => {
                    break;
                }
            }
        }
        
        // 清理所有活跃任务
        let mut active_tasks = self.active_tasks.lock().await;
        for (_, handle) in active_tasks.drain() {
            handle.abort();
        }
    }
    
    /// 处理添加上传任务
    async fn handle_add_upload(
        &self,
        file_path: PathBuf,
        strategy: UploadStrategy,
        metadata: HashMap<String, String>,
    ) -> Result<UploadId> {
        // 检查文件是否存在
        if !file_path.exists() {
            return Err(TransferError::not_found(format!("File not found: {:?}", file_path)));
        }
        
        // 获取文件大小
        let file_size = tokio::fs::metadata(&file_path)
            .await?
            .len();
        
        // 创建任务
        let upload_id = UploadId::new();
        let task = UploadTask {
            id: upload_id,
            file_path,
            file_size,
            strategy,
            metadata,
            state: UploadState::Queued,
            uploaded_bytes: 0,
            upload_url: None,
            created_at: chrono::Utc::now(),
            started_at: None,
            completed_at: None,
            error: None,
            retry_count: 0,
        };
        
        // 保存任务
        {
            let mut tasks = self.tasks.write().await;
            tasks.insert(upload_id, task.clone());
        }
        
        // 发送事件
        let _ = self.event_tx.send(UploadEvent::TaskAdded { upload_id });
        
        // 保存到存储
        if let Some(storage) = &self.storage_adapter {
            storage.save_task(&task).await?;
        }
        
        // 检查并启动任务
        self.check_and_start_tasks().await;
        
        Ok(upload_id)
    }
    
    /// 处理暂停任务
    async fn handle_pause(&self, upload_id: UploadId) -> Result<()> {
        // 更新任务状态
        let mut tasks = self.tasks.write().await;
        let task = tasks.get_mut(&upload_id)
            .ok_or_else(|| TransferError::not_found(format!("Task {} not found", upload_id)))?;
        
        if task.state != UploadState::Uploading {
            return Err(TransferError::invalid_state("Can only pause uploading tasks"));
        }
        
        let old_state = task.state;
        task.state = UploadState::Paused;
        
        // 发送状态变更事件
        let _ = self.event_tx.send(UploadEvent::StateChanged {
            upload_id,
            old_state,
            new_state: UploadState::Paused,
        });
        
        // 取消活跃任务
        let mut active_tasks = self.active_tasks.lock().await;
        if let Some(handle) = active_tasks.remove(&upload_id) {
            handle.abort();
        }
        
        Ok(())
    }
    
    /// 处理恢复任务
    async fn handle_resume(&self, upload_id: UploadId) -> Result<()> {
        // 更新任务状态
        let mut tasks = self.tasks.write().await;
        let task = tasks.get_mut(&upload_id)
            .ok_or_else(|| TransferError::not_found(format!("Task {} not found", upload_id)))?;
        
        if task.state != UploadState::Paused {
            return Err(TransferError::invalid_state("Can only resume paused tasks"));
        }
        
        let old_state = task.state;
        task.state = UploadState::Queued;
        
        // 发送状态变更事件
        let _ = self.event_tx.send(UploadEvent::StateChanged {
            upload_id,
            old_state,
            new_state: UploadState::Queued,
        });
        
        drop(tasks);
        
        // 检查并启动任务
        self.check_and_start_tasks().await;
        
        Ok(())
    }
    
    /// 处理取消任务
    async fn handle_cancel(&self, upload_id: UploadId) -> Result<()> {
        // 更新任务状态
        let mut tasks = self.tasks.write().await;
        let task = tasks.get_mut(&upload_id)
            .ok_or_else(|| TransferError::not_found(format!("Task {} not found", upload_id)))?;
        
        let old_state = task.state;
        task.state = UploadState::Cancelled;
        
        // 发送状态变更事件
        let _ = self.event_tx.send(UploadEvent::StateChanged {
            upload_id,
            old_state,
            new_state: UploadState::Cancelled,
        });
        
        // 取消活跃任务
        let mut active_tasks = self.active_tasks.lock().await;
        if let Some(handle) = active_tasks.remove(&upload_id) {
            handle.abort();
        }
        
        Ok(())
    }
    
    /// 检查并启动排队的任务
    async fn check_and_start_tasks(&self) {
        let active_count = self.active_tasks.lock().await.len();
        if active_count >= self.config.max_concurrent {
            return;
        }
        
        let tasks = self.tasks.read().await;
        let mut queued_tasks: Vec<_> = tasks
            .values()
            .filter(|t| t.state == UploadState::Queued)
            .cloned()
            .collect();
        
        // 按创建时间排序
        queued_tasks.sort_by_key(|t| t.created_at);
        
        drop(tasks);
        
        let slots_available = self.config.max_concurrent - active_count;
        for task in queued_tasks.into_iter().take(slots_available) {
            self.start_upload_task(task).await;
        }
    }
    
    /// 启动上传任务
    async fn start_upload_task(&self, task: UploadTask) {
        let upload_id = task.id;
        
        // 更新任务状态
        {
            let mut tasks = self.tasks.write().await;
            if let Some(t) = tasks.get_mut(&upload_id) {
                t.state = UploadState::Preparing;
                t.started_at = Some(chrono::Utc::now());
            }
        }
        
        // 获取上传器
        let uploader = match self.get_uploader_for_strategy(&task.strategy).await {
            Ok(u) => u,
            Err(e) => {
                self.handle_task_error(upload_id, e).await;
                return;
            }
        };
        
        // 创建上传任务
        let manager = self.clone();
        
        // 使用 spawn_blocking 来避免 Send 问题
        let handle = {
            let task = task;
            let uploader = uploader;
            tokio::spawn(async move {
                manager.run_upload_task(task, uploader).await;
            })
        };
        
        // 保存任务句柄
        let mut active_tasks = self.active_tasks.lock().await;
        active_tasks.insert(upload_id, handle);
    }
    
    /// 运行上传任务
    async fn run_upload_task(&self, mut task: UploadTask, uploader: Arc<dyn Uploader + Send + Sync>) {
        // 更新状态为上传中
        {
            let mut tasks = self.tasks.write().await;
            if let Some(t) = tasks.get_mut(&task.id) {
                t.state = UploadState::Uploading;
            }
        }
        
        let _ = self.event_tx.send(UploadEvent::StateChanged {
            upload_id: task.id,
            old_state: UploadState::Preparing,
            new_state: UploadState::Uploading,
        });
        
        // 执行上传
        let result = if task.upload_url.is_some() && uploader.supports_resume() {
            uploader.resume(&task).await
        } else {
            uploader.upload(&task).await
        };
        
        match result {
            Ok(url) => {
                self.handle_task_completed(task.id, url).await;
            }
            Err(e) => {
                self.handle_task_error(task.id, e).await;
            }
        }
        
        // 移除活跃任务
        let mut active_tasks = self.active_tasks.lock().await;
        active_tasks.remove(&task.id);
        
        // 检查并启动下一个任务
        drop(active_tasks);
        self.check_and_start_tasks().await;
    }
    
    /// 处理任务完成
    async fn handle_task_completed(&self, upload_id: UploadId, url: String) {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(&upload_id) {
            task.state = UploadState::Completed;
            task.upload_url = Some(url.clone());
            task.completed_at = Some(chrono::Utc::now());
            
            // 发送完成事件
            let _ = self.event_tx.send(UploadEvent::Completed {
                upload_id,
                url: url.clone(),
            });
            
            // 调用回调
            if let Some(callback) = &self.progress_callback {
                tokio::spawn({
                    let callback = callback.clone();
                    async move {
                        callback.on_completed(upload_id, &url).await;
                    }
                });
            }
        }
    }
    
    /// 处理任务错误
    async fn handle_task_error(&self, upload_id: UploadId, error: TransferError) {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(&upload_id) {
            task.state = UploadState::Failed;
            task.error = Some(error.to_string());
            
            // 发送失败事件
            let _ = self.event_tx.send(UploadEvent::Failed {
                upload_id,
                error: error.to_string(),
            });
            
            // 调用回调
            if let Some(callback) = &self.progress_callback {
                tokio::spawn({
                    let callback = callback.clone();
                    let error_msg = error.to_string();
                    async move {
                        callback.on_failed(upload_id, &error_msg).await;
                    }
                });
            }
        }
    }
    
    /// 根据策略获取上传器
    async fn get_uploader_for_strategy(&self, strategy: &UploadStrategy) -> Result<Arc<dyn Uploader + Send + Sync>> {
        let uploaders = self.uploaders.read().await;
        
        let uploader_name = match strategy {
            UploadStrategy::Tus(_) => "tus",
            UploadStrategy::Simple(_) => "simple",
            UploadStrategy::Chunked(_) => "chunked",
        };
        
        uploaders
            .get(uploader_name)
            .cloned()
            .ok_or_else(|| TransferError::not_found(format!("Uploader '{}' not found", uploader_name)))
    }
}

impl Clone for TransferManager {
    fn clone(&self) -> Self {
        Self {
            uploaders: self.uploaders.clone(),
            tasks: self.tasks.clone(),
            active_tasks: self.active_tasks.clone(),
            command_tx: self.command_tx.clone(),
            event_tx: self.event_tx.clone(),
            config: self.config.clone(),
            progress_callback: self.progress_callback.clone(),
            storage_adapter: self.storage_adapter.clone(),
        }
    }
}

/// 传输管理器构建器
pub struct TransferManagerBuilder {
    config: TransferConfig,
    uploaders: HashMap<String, Arc<dyn Uploader + Send + Sync>>,
    progress_callback: Option<Arc<dyn ProgressCallback + Send + Sync>>,
    storage_adapter: Option<Arc<dyn StorageAdapter + Send + Sync>>,
}

impl TransferManagerBuilder {
    pub fn new() -> Self {
        Self {
            config: TransferConfig::default(),
            uploaders: HashMap::new(),
            progress_callback: None,
            storage_adapter: None,
        }
    }
    
    pub fn config(mut self, config: TransferConfig) -> Self {
        self.config = config;
        self
    }
    
    pub fn register_uploader(mut self, name: &str, uploader: Arc<dyn Uploader + Send + Sync>) -> Self {
        self.uploaders.insert(name.to_string(), uploader);
        self
    }
    
    pub fn progress_callback(mut self, callback: Arc<dyn ProgressCallback + Send + Sync>) -> Self {
        self.progress_callback = Some(callback);
        self
    }
    
    pub fn storage_adapter(mut self, adapter: Arc<dyn StorageAdapter + Send + Sync>) -> Self {
        self.storage_adapter = Some(adapter);
        self
    }
    
    pub async fn build(self) -> Result<(TransferManager, JoinHandle<()>)> {
        let (mut manager, handle) = TransferManager::new(self.config);
        
        // 注册上传器
        for (name, uploader) in self.uploaders {
            manager.register_uploader(&name, uploader).await?;
        }
        
        // 设置回调和存储
        if let Some(callback) = self.progress_callback {
            manager.set_progress_callback(callback);
        }
        
        if let Some(adapter) = self.storage_adapter {
            manager.set_storage_adapter(adapter);
        }
        
        Ok((manager, handle))
    }
}
