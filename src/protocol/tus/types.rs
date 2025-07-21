use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct TusConfig {
    pub endpoint: String,
    pub chunk_size: usize,
    pub timeout: std::time::Duration,
    pub max_retries: u8,
    pub retry_delay: std::time::Duration,
    pub tus_version: String,
    pub use_creation_extension: bool,
    pub headers: HashMap<String, String>,
}

impl Default for TusConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            chunk_size: 5 * 1024 * 1024, // 5MB
            timeout: std::time::Duration::from_secs(30),
            max_retries: 3,
            retry_delay: std::time::Duration::from_secs(1),
            tus_version: "1.0.0".to_string(),
            use_creation_extension: true,
            headers: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TusUploadOptions {
    pub metadata: HashMap<String, String>,
    /// Delete origin file when upload completed
    pub delete_on_termination: bool,
    /// unit: seconds
    pub upload_expires_in: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum TusError {
    ProtocolError(String),
    ServerError {
        status: u16,
        message: String,
    },
    UploadIncomplete {
        expected: u64,
        actual: u64,
    },
    InvalidOffset {
        expected: u64,
        actual: u64,
    }
}

impl std::fmt::Display for TusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProtocolError(msg) => write!(f, "TUS protocol error: {}", msg),
            Self::ServerError { status, message } => {
                write!(f, "TUS server error ({}): {}", status, message)
            }
            Self::UploadIncomplete { expected, actual } => {
                write!(f, "Upload incomplete: expected {} bytes, got {}", expected, actual)
            }
            Self::InvalidOffset { expected, actual } => {
                write!(f, "Invalid offset: expected {}, got {}", expected, actual)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TusMetadata {
    pub upload_url: String,
    pub upload_offset: u64,
    /// file size
    pub upload_length: u64,
    pub metadata: HashMap<String, String>,
    pub extensions: Vec<String>,
    pub upload_expires: Option<String>,
}

impl std::error::Error for TusError {}
