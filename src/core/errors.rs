use thiserror::Error;

#[derive(Error, Debug)]
pub enum TransferError {
    #[error("IO error {0}")]
    Io(#[from] std::io::Error),

    #[error("Network error {0}")]
    Network(String),

    #[error("HTTP error {0}")]
    Http(#[from] reqwest::Error),

    #[error("Protocol error {0}")]
    Protocol(String),

    #[error("Invalid header {0}")]
    InvalidHeader(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Manager shutdown")]
    ManagerShutdown,

    #[error("URL prase error: {0}")]
    UrlParseError(#[from] url::ParseError),
    
    #[error("Transfer incomplete: expected {expected} bytes, got {actual} bytes")]
    Incomplete {
        expected: u64,
        actual: u64,
    },

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Protocol specific error: {protocol} - {message}")]
    ProtocolSpecific {
        protocol: String,
        message: String,
    },
}

impl TransferError {
    pub fn protocol_specific(protocol: impl Into<String>, message: impl Into<String>) -> Self {
        Self::ProtocolSpecific {
            protocol: protocol.into(),
            message: message.into(),
        }
    }

    pub fn protocol(protocol: impl Into<String>) -> Self {
        Self::Protocol(protocol.into())
    }
}

impl From<reqwest::header::InvalidHeaderValue> for TransferError {
    fn from(err: reqwest::header::InvalidHeaderValue) -> Self {
        Self::InvalidHeader(err.to_string())
    }
}

impl From<reqwest::header::InvalidHeaderName> for TransferError {
    fn from(err: reqwest::header::InvalidHeaderName) -> Self {
        Self::InvalidHeader(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, TransferError>;
