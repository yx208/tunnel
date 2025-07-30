use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransferError {
    #[error("Http error: {0}")]
    Http(#[from] reqwest::Error)
}

pub type Result<T> = std::result::Result<T, TransferError>;
