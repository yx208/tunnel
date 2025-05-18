use reqwest::StatusCode;
use thiserror::Error;
use url::ParseError;

pub mod client;
mod constant;

#[derive(Error, Debug)]
pub enum TusError {
    #[error("Unable to get file name")]
    NotFileName,

    #[error("The file name contains invalid UTF-8 characters")]
    InvalidFileName,

    #[error(transparent)]
    IO(#[from] std::io::Error),

    #[error(transparent)]
    UrlParseError(#[from] ParseError),

    #[error(transparent)]
    InvalidHeaderValue(#[from] reqwest::header::InvalidHeaderValue),

    #[error(transparent)]
    Request(#[from] reqwest::Error),

    #[error("No upload location returned from server")]
    NotLocation,

    #[error("No upload offset returned from server")]
    NotOffset,

    #[error("{message}: {status}")]
    UnexpectedStatus {
        message: String,
        status: StatusCode,
    },

    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error>),
}
