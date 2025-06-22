use thiserror::Error;
use std::io;

pub type Result<T> = std::result::Result<T, TransferError>;

#[derive(Error, Debug)]
pub enum TransferError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    #[error("Upload not found: {0}")]
    NotFound(String),

    #[error("Upload already exists: {0}")]
    AlreadyExists(String),

    #[error("Invalid state transition: {0}")]
    InvalidState(String),

    #[error("Server error ({status_code}): {message}")]
    ServerError {
        status_code: u16,
        message: String,
    },

    #[error("Upload incomplete: expected {expected} bytes, got {actual} bytes")]
    UploadIncomplete {
        expected: u64,
        actual: u64,
    },

    #[error("Operation cancelled")]
    Cancelled,

    #[error("Timeout")]
    Timeout,

    #[error("Retry limit exceeded")]
    RetryLimitExceeded,

    #[error("Unsupported operation: {0}")]
    Unsupported(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("{0}")]
    Custom(String),
}

impl TransferError {
    pub fn invalid_parameter(msg: impl Into<String>) -> Self {
        Self::InvalidParameter(msg.into())
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }

    pub fn already_exists(msg: impl Into<String>) -> Self {
        Self::AlreadyExists(msg.into())
    }

    pub fn invalid_state(msg: impl Into<String>) -> Self {
        Self::InvalidState(msg.into())
    }

    pub fn server_error(status_code: u16, message: impl Into<String>) -> Self {
        Self::ServerError {
            status_code,
            message: message.into(),
        }
    }

    pub fn unsupported(msg: impl Into<String>) -> Self {
        Self::Unsupported(msg.into())
    }

    pub fn storage(msg: impl Into<String>) -> Self {
        Self::Storage(msg.into())
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    pub fn custom(msg: impl Into<String>) -> Self {
        Self::Custom(msg.into())
    }
}
