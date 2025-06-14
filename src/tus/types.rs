use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
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

impl std::fmt::Display for UploadId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
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

/// 单个任务的进度信息
#[derive(Debug, Clone)]
pub struct TaskProgress {
    pub upload_id: UploadId,
    pub bytes_uploaded: u64,
    pub total_bytes: u64,
    pub instant_speed: f64,
    pub average_speed: f64,
    pub percentage: f64,
    pub eta: Option<Duration>,
}

/// 聚合更新统计信息
#[derive(Debug, Clone)]
pub struct AggregatedStats {
    /// 总任务数
    pub total_tasks: usize,

    /// 活跃的任务数（未完成）
    pub active_tasks: usize,

    /// 所有任务的总字节数
    pub total_bytes: u64,

    /// 所有任务已上传的总字节数
    pub total_uploaded: u64,

    /// 总体速度（所有任务的总和）
    pub overall_speed: f64,

    /// 总体百分比
    pub overall_percentage: f64,

    /// 预计所有任务完成时间
    pub overall_eta: Option<Duration>,
}

/// 批量进度更新
#[derive(Debug, Clone)]
pub struct BatchProgress {
    /// 更新时间戳
    pub timestamp: Instant,

    /// 各个任务进度
    pub tasks: Vec<TaskProgress>,

    /// 聚合统计信息
    pub aggregated: AggregatedStats,
}

#[derive(Debug, Clone)]
pub enum UploadEvent {
    /// 任务状态变更
    StateChanged {
        upload_id: UploadId,
        old_state: UploadState,
        new_state: UploadState,
    },

    /// 批量更新进度
    BatchProgress(BatchProgress),

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

    /// 所有任务完成
    AllCompleted {
        total_tasks: usize,
        total_bytes: u64,
        total_duration: Duration,
    }
}

/// 上传管理器命令
pub enum ManagerCommand {
    /// 添加上传任务
    AddUpload {
        file_path: PathBuf,
        metadata: Option<HashMap<String, String>>,
        reply: oneshot::Sender<Result<UploadId>>,
    },

    /// 批量添加
    BatchAdd {
        files: Vec<(PathBuf, Option<HashMap<String, String>>)>,
        reply: oneshot::Sender<Result<Vec<UploadId>>>,
    },

    /// 暂停
    PauseUpload {
        upload_id: UploadId,
        reply: oneshot::Sender<Result<()>>,
    },

    /// 暂停所有
    PauseAll {
        reply: oneshot::Sender<Result<()>>,
    },

    /// 恢复
    ResumeUpload {
        upload_id: UploadId,
        reply: oneshot::Sender<Result<()>>,
    },

    /// 恢复所有
    ResumeAll {
        reply: oneshot::Sender<Result<()>>,
    },

    /// 取消
    CancelUpload {
        upload_id: UploadId,
        reply: oneshot::Sender<Result<()>>,
    },

    /// 取消所有
    CancelAll {
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
        reply: oneshot::Sender<usize>,
    },

    /// 设置进度更新间隔
    SetProgressInterval {
        interval: Duration,
        reply: oneshot::Sender<Result<()>>,
    }
}

#[derive(Debug, Clone)]
pub struct UploadConfig {
    /// 最大并发数
    pub concurrent: usize,

    /// 进度更新间隔
    pub progress_interval: Duration,

    /// 是否启用批量进度更新
    pub batch_progress: bool,

    /// 分块大小
    pub chunk_size: usize,

    /// 重试次数
    pub max_retries: u8,

    /// 重试间隔
    pub retry_delay: Duration,

    /// 是否自动保存状态
    pub auto_save_state: bool,

    /// 任务状态文件路径
    pub state_file: Option<PathBuf>,
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            concurrent: 3,
            progress_interval: Duration::from_secs(1),
            batch_progress: true,
            chunk_size: 5 * 1024 * 1024, // 5MB
            max_retries: 3,
            retry_delay: Duration::from_secs(5),
            auto_save_state: true,
            state_file: None,
        }
    }
}

