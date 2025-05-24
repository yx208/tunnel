use thiserror::Error;

#[derive(Error, Debug)]
pub enum TusError {
    #[error("HTTP Request error: {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Context error: {0}")]
    ContextError(#[from] anyhow::Error),

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
    
    #[error("To str error: {0}")]
    ToStrError(#[from] reqwest::header::ToStrError),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Failed to parse response header '{header_name}': {message}")]
    HeaderParseError {
        header_name: String,
        message: String,
    },

    #[error("Param error: {0}")]
    ParamError(String),

    #[error("Integer parse error: {0}")]
    IntParseError(#[from] std::num::ParseIntError),

    #[error("Invalid header value: {0}")]
    InvalidHeaderValue(#[from] reqwest::header::InvalidHeaderValue),

    #[error("Invalid header name: {0}")]
    InvalidHeaderName(#[from] reqwest::header::InvalidHeaderName),
    
    #[error("Invalid file: {0}")]
    InvalidFile(String),

    #[error("Upload was cancelled")]
    Cancelled,

    #[error("Operation error: {0}")]
    OperationError(String),
    
    #[error("Internal error: {0}")]
    InternalError(String),
}

impl TusError {
    pub fn server_error(status_code: u16, message: impl Into<String>) -> Self {
        Self::ServerError {
            status_code,
            message: message.into(),
        }
    }

    pub fn invalid_response(message: impl Into<String>) -> Self {
        Self::InvalidResponse(message.into())
    }

    pub fn header_parse_error(header_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self::HeaderParseError {
            header_name: header_name.into(),
            message: message.into(),
        }
    }

    pub fn operation_error(message: impl Into<String>) -> Self {
        Self::OperationError(message.into())
    }
}

/// Error alias
pub type Result<T> = std::result::Result<T, TusError>;
