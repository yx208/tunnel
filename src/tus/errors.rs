use thiserror::Error;

#[derive(Error, Debug)]
pub enum TusError {
    #[error("HTTP Request error: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Server error: status code {status_code}, message: {message}")]
    ServerError {
        status_code: u16,
        message: String,
    },

    #[error("Upload incomplete expected: {expected}, actual: {actual}")]
    UploadIncomplete {
        expected: u64,
        actual: u64,
    },

    #[error("Param error: {0}")]
    ParamError(String),

    #[error("Invalid header value: {0}")]
    InvalidHeaderValue(#[from] reqwest::header::InvalidHeaderValue),

    #[error("Invalid header name: {0}")]
    InvalidHeaderName(#[from] reqwest::header::InvalidHeaderName),

    #[error("Upload was cancelled")]
    Cancelled,

    #[error("Internal error: {0}")]
    Internal(String),
}

impl TusError {
    pub fn server_error(status_code: u16, message: impl Into<String>) -> Self {
        Self::ServerError {
            status_code,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
}

impl From<serde_json::Error> for TusError {
    fn from(err: serde_json::Error) -> Self {
        TusError::Internal(err.to_string())
    }
}

/// Error alias
pub type Result<T, E = TusError> = std::result::Result<T, E>;
