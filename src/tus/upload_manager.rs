use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, mpsc, RwLock};
use uuid::Uuid;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use crate::tus::manager_worker::UploadManagerWorker;
use crate::tus::types::TusClient;
use super::errors::{Result, TusError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct UploadId(Uuid);

impl UploadId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum UploadState {
    /// 等待中（在队列中）
    Queued,
    /// 准备中（创建上传会话）
    Preparing,
    /// 上传中
    Uploading,
    /// 已暂停
    Paused,
    /// 已完成
    Completed,
    /// 失败
    Failed,
    /// 已取消
    Cancelled,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UploadTask {
    pub id: UploadId,
    pub file_path: PathBuf,
    pub file_size: u64,
    pub upload_url: Option<String>,
    pub state: UploadState,
    pub bytes_uploaded: u64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone)]
pub struct UploadProgress {
    pub upload_id: UploadId,
    pub bytes_uploaded: u64,
    pub total_bytes: u64,
    pub instant_speed: f64,
    pub average_speed: f64,
    pub percentage: f64,
    pub eta: Option<Duration>,
}

#[derive(Debug, Clone)]
pub enum UploadEvent {
    /// 任务状态变更
    StateChanged {
        upload_id: UploadId,
        old_state: UploadState,
        new_state: UploadState,
    },

    /// 进度更新
    Progress(UploadProgress),

    /// 任务失败
    Failed {
        upload_id: UploadId,
        error: String,
    },

    /// 任务完成
    Completed {
        upload_id: UploadId,
        upload_url: String,
    },
}

/// 上传管理器命令
pub enum ManagerCommand {
    /// 添加上传任务
    AddUpload {
        file_path: PathBuf,
        metadata: Option<HashMap<String, String>>,
        reply: oneshot::Sender<Result<UploadId>>,
    },

    /// 暂停
    PauseUpload {
        upload_id: UploadId,
        reply: oneshot::Sender<Result<()>>,
    },

    /// 恢复
    ResumeUpload {
        upload_id: UploadId,
        reply: oneshot::Sender<Result<()>>,
    },

    /// 取消
    CancelUpload {
        upload_id: UploadId,
        reply: oneshot::Sender<Result<()>>,
    },

    /// 获取任务信息
    GetTask {
        upload_id: UploadId,
        reply: oneshot::Sender<Option<UploadTask>>,
    },

    /// 获取所有任务
    GetAllTasks {
        reply: oneshot::Sender<Vec<UploadTask>>,
    }
}

pub struct UploadManager {
    command_tx: mpsc::Sender<ManagerCommand>,
    event_rx: Arc<RwLock<mpsc::UnboundedReceiver<UploadEvent>>>,
    worker_handle: JoinHandle<()>,
}

impl UploadManager {
    pub fn new(client: TusClient, concurrent: usize, state_file: Option<PathBuf>) -> Self {
        let (command_tx, command_rx) = mpsc::channel(100);
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let worker_handle = tokio::spawn(UploadManagerWorker::run(
            client,
            concurrent,
            state_file,
            command_rx,
            event_tx
        ));

        Self {
            command_tx,
            event_rx: Arc::new(RwLock::new(event_rx)),
            worker_handle,
        }
    }

    /// Add upload task
    pub async fn add_upload(&self, file_path: PathBuf, metadata: Option<HashMap<String, String>>)
        -> Result<UploadId>
    {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(ManagerCommand::AddUpload {
                file_path: file_path.into(),
                metadata,
                reply: reply_tx
            })
            .await
            .map_err(|_| TusError::InternalError("Send command failed".to_string()))?;

        // 等待响应
        reply_rx
            .await
            .map_err(|_| TusError::InternalError("Failed to add upload".to_string()))?
    }

    /// Pause upload task
    pub async fn pause_upload(&self, upload_id: UploadId) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        
        self.command_tx
            .send(ManagerCommand::PauseUpload {
                upload_id,
                reply: reply_tx
            })
            .await
            .map_err(|_| TusError::InternalError("Send command [PauseUpload] failed".to_string()))?;
        
        reply_rx
            .await
            .map_err(|_| TusError::InternalError("Failed to pause upload".to_string()))?;
        
        Ok(())
    }
}
