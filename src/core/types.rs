use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use uuid::Uuid;

// 用于序列化 Duration
fn serialize_duration<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_u64(duration.as_secs())
}

fn deserialize_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let secs = u64::deserialize(deserializer)?;
    Ok(Duration::from_secs(secs))
}



/// 上传任务唯一标识
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct UploadId(pub Uuid);

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

/// 上传状态枚举
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

/// 上传策略
#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum UploadStrategy {
    /// Tus 协议上传
    Tus(TusConfig),
    /// 简单 HTTP 上传
    Simple(SimpleConfig),
    /// 分片上传
    Chunked(ChunkedConfig),
}

/// Tus 配置
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TusConfig {
    pub chunk_size: usize,
    pub parallel_chunks: bool,
    pub max_retries: u32,
}

impl Default for TusConfig {
    fn default() -> Self {
        Self {
            chunk_size: 5 * 1024 * 1024, // 5MB
            parallel_chunks: false,
            max_retries: 3,
        }
    }
}

/// 简单上传配置
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SimpleConfig {
    #[serde(serialize_with = "serialize_duration", deserialize_with = "deserialize_duration")]
    pub timeout: Duration,
    pub max_retries: u32,
}

impl Default for SimpleConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(300), // 5 分钟
            max_retries: 3,
        }
    }
}

/// 分片上传配置
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChunkedConfig {
    pub chunk_size: usize,
    pub concurrent_chunks: usize,
    pub max_retries: u32,
}

impl Default for ChunkedConfig {
    fn default() -> Self {
        Self {
            chunk_size: 5 * 1024 * 1024, // 5MB
            concurrent_chunks: 3,
            max_retries: 3,
        }
    }
}

/// 上传任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadTask {
    /// 任务 ID
    pub id: UploadId,
    /// 文件路径
    pub file_path: PathBuf,
    /// 文件大小
    pub file_size: u64,
    /// 上传策略
    pub strategy: UploadStrategy,
    /// 元数据
    pub metadata: HashMap<String, String>,
    /// 当前状态
    pub state: UploadState,
    /// 已上传字节数
    pub uploaded_bytes: u64,
    /// 上传 URL（如果有）
    pub upload_url: Option<String>,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 开始时间
    pub started_at: Option<DateTime<Utc>>,
    /// 完成时间
    pub completed_at: Option<DateTime<Utc>>,
    /// 错误信息
    pub error: Option<String>,
    /// 重试次数
    pub retry_count: u32,
}

/// 上传进度
#[derive(Debug, Clone)]
pub struct UploadProgress {
    /// 已上传字节数
    pub uploaded_bytes: u64,
    /// 总字节数
    pub total_bytes: u64,
    /// 当前速度（字节/秒）
    pub speed: f64,
    /// 平均速度（字节/秒）
    pub average_speed: f64,
    /// 完成百分比
    pub percentage: f64,
    /// 预计剩余时间
    pub eta: Option<Duration>,
}

/// 传输管理器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferConfig {
    /// 最大并发任务数
    pub max_concurrent: usize,
    /// 任务队列大小
    pub queue_size: usize,
    /// 进度更新间隔
    #[serde(serialize_with = "serialize_duration", deserialize_with = "deserialize_duration")]
    pub progress_interval: Duration,
    /// 是否启用状态持久化
    pub enable_persistence: bool,
    /// 状态文件路径
    pub state_file: Option<PathBuf>,
    /// 默认上传策略
    pub default_strategy: UploadStrategy,
}

impl Default for TransferConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 3,
            queue_size: 100,
            progress_interval: Duration::from_millis(500),
            enable_persistence: true,
            state_file: None,
            default_strategy: UploadStrategy::Tus(TusConfig::default()),
        }
    }
}

/// 管理器命令
pub enum ManagerCommand {
    /// 添加上传任务
    AddUpload {
        file_path: PathBuf,
        strategy: UploadStrategy,
        metadata: HashMap<String, String>,
        reply: oneshot::Sender<crate::core::errors::Result<UploadId>>,
    },
    /// 暂停任务
    Pause {
        upload_id: UploadId,
        reply: oneshot::Sender<crate::core::errors::Result<()>>,
    },
    /// 恢复任务
    Resume {
        upload_id: UploadId,
        reply: oneshot::Sender<crate::core::errors::Result<()>>,
    },
    /// 取消任务
    Cancel {
        upload_id: UploadId,
        reply: oneshot::Sender<crate::core::errors::Result<()>>,
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
    /// 关闭管理器
    Shutdown,
}

/// 上传事件
#[derive(Debug, Clone)]
pub enum UploadEvent {
    /// 任务已添加
    TaskAdded {
        upload_id: UploadId,
    },
    /// 状态变更
    StateChanged {
        upload_id: UploadId,
        old_state: UploadState,
        new_state: UploadState,
    },
    /// 进度更新
    Progress {
        upload_id: UploadId,
        progress: UploadProgress,
    },
    /// 任务完成
    Completed {
        upload_id: UploadId,
        url: String,
    },
    /// 任务失败
    Failed {
        upload_id: UploadId,
        error: String,
    },
}

// 静态断言确保类型是 Send的
const _: () = {
    fn assert_send<T: Send>() {}
    fn assert_types() {
        assert_send::<UploadTask>();
        assert_send::<UploadEvent>();
        assert_send::<UploadProgress>();
    }
};
