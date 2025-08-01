use reqwest::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransferError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Manager is shutdown")]
    ManagerShutdown,

    #[error("Protocol error: {protocol} - {message}")]
    Protocol {
        protocol: String,
        message: String,
    },

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    
    #[error("URL parse error: {0}")]
    URL(#[from] url::ParseError),

    #[error("Other error: {0}")]
    Other(String),
}

impl TransferError {
    pub fn protocol_specific(protocol: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Protocol {
            protocol: protocol.into(),
            message: message.into(),
        }
    }
}

impl From<InvalidHeaderValue> for TransferError {
    fn from(value: InvalidHeaderValue) -> Self {
        Self::Other(format!("Invalid header value - {}", value))
    }
}

impl From<InvalidHeaderName> for TransferError {
    fn from(value: InvalidHeaderName) -> Self {
        Self::Other(format!("Invalid header name - {}", value))
    }
}

pub type Result<T> = std::result::Result<T, TransferError>;
