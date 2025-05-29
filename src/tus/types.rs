use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use uuid::Uuid;
use super::errors::Result;
use super::task::UploadTask;

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
    },

    /// 清除所有 <Failed/Completed> 状态的任务
    Clean {
        reply: oneshot::Sender<Result<()>>,
    }
}
