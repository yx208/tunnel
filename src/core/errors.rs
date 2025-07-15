use thiserror::Error;

#[derive(Error, Debug)]
pub enum TransferError {
    #[error("IO error {0}")]
    Io(#[from] std::io::Error),

    #[error("Network error {0}")]
    Network(String),

    #[error("HTTP error {0}")]
    Http(#[from] reqwest::Error),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Manager shutdown")]
    ManagerShutdown,
}

pub type Result<T> = std::result::Result<T, TransferError>;
