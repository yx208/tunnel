use std::fmt::Debug;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransferId(Uuid);

impl TransferId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl std::fmt::Display for TransferId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub enum TransferState {
    Queued,
    Preparing,
    Transferring,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl TransferState {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Preparing | Self::Transferring)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone)]
pub struct TransferConfig {
    pub max_concurrent: usize,
    pub buffer_size: usize,
    pub progress_interval: Duration,
    pub batch_progress: bool,
    pub retry_config: RetryConfig,
    pub rate_limit: Option<u64>,
    pub temp_dir: Option<PathBuf>,
    pub auto_save_state: bool,
    pub state_file: Option<PathBuf>,
    pub verification: VerificationConfig,
}

impl Default for TransferConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 3,
            buffer_size: 8 * 1024 * 1024,
            progress_interval: Duration::from_secs(1),
            batch_progress: true,
            retry_config: RetryConfig::default(),
            rate_limit: None,
            temp_dir: None,
            auto_save_state: false,
            state_file: None,
            verification: VerificationConfig::default(),
        }
    }
}

/// Transfer statistical information
#[derive(Debug, Clone)]
pub struct TransferStats {
    pub start_time: Instant,
    pub end_time: Option<Instant>,
    pub bytes_transferred: u64,
    pub total_bytes: Option<u64>,
    pub current_speed: f64,
    pub average_speed: f64,
    pub eta: Option<Duration>,
    pub retry_count: u8,
}

impl TransferStats {
    pub fn new(total_bytes: Option<u64>) -> Self {
        Self {
            start_time: Instant::now(),
            end_time: None,
            bytes_transferred: 0,
            total_bytes,
            current_speed: 0.0,
            average_speed: 0.0,
            eta: None,
            retry_count: 0,
        }
    }

    pub fn progress_percentage(&self) -> f64 {
        if let Some(total) = self.total_bytes {
            if total > 0 {
                (self.bytes_transferred as f64 / total as f64) * 100.0
            } else {
                0.0
            }
        } else {
            0.0
        }
    }

    pub fn elapsed(&self) -> Duration {
        if let Some(end_time) = self.end_time {
            end_time - self.start_time
        } else {
            Instant::now() - self.start_time
        }
    }
}

#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u8,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub multiplier: f64,
    /// Whether to retry the temporary error
    pub retry_on_temporary: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
            retry_on_temporary: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct VerificationConfig {
    pub verify_checksum: bool,
    pub checksum_algorithm: Option<ChecksumAlgorithm>,
    pub verify_size: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum ChecksumAlgorithm {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

#[derive(Debug, Clone)]
pub enum TransferEvent {
    StateChange {
        id: TransferId,
        old_state: TransferState,
        new_state: TransferState,
    },

    Progress {
        id: TransferId,
        stats: TransferStats,
    },

    BatchProgress {
        updates: Vec<(TransferId, TransferStats)>,
    },

    Started {
        id: TransferId,
    },

    Completed {
        id: TransferId,
        stats: TransferStats,
    },

    Failed {
        id: TransferId,
        error: String,
        stats: TransferStats,
    },

    Cancelled {
        id: TransferId,
    },

    Retry {
        id: TransferId,
        attempt: u8,
        reason: String,
    },
}

/// Task priority
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Priority(pub u8);

impl Priority {
    pub const LOW: Priority = Priority(0);
    pub const NORMAL: Priority = Priority(128);
    pub const HIGH: Priority = Priority(255);
}

impl Default for Priority {
    fn default() -> Self {
        Self::NORMAL
    }
}

#[derive(Debug, Clone)]
pub struct TransferOptions {
    pub priority: Priority,
    pub metadata: serde_json::Value,
    pub tags: Vec<String>,
    pub auto_start: bool,
    pub config_override: Option<TransferConfig>,
}

impl Default for TransferOptions {
    fn default() -> Self {
        Self {
            priority: Priority::default(),
            metadata: serde_json::Value::Null,
            tags: Vec::new(),
            auto_start: true,
            config_override: None,
        }
    }
}
