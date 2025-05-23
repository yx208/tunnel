use std::path::Path;
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use crate::config::get_config;
use crate::tus::constants::TUS_RESUMABLE;
use crate::tus::errors::{Result, TusError};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UploadStrategy {
    Chunked,
    Streaming
}

#[derive(Debug, Clone)]
pub struct TusClientConfig {
    pub endpoint: String,
    pub chunk_size: usize,
    pub strategy: UploadStrategy,
    pub speed_limit: Option<u64>,
    pub timeout: u64,
    pub tcp_nodelay: bool,
}

impl Default for TusClientConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            chunk_size: super::constants::DEFAULT_CHUNK_SIZE,
            strategy: UploadStrategy::Chunked,
            speed_limit: super::constants::DEFAULT_MAX_SPEED,
            timeout: super::constants::DEFAULT_TIMEOUT,
            tcp_nodelay: true,
        }
    }
}

pub struct TusClient {
    pub client: Client,
    pub endpoint: String,
    pub chunk_size: usize,
    pub strategy: UploadStrategy,
    pub speed_limit: Option<u64>,
}

impl TusClient {
    pub fn create_headers() -> HeaderMap {
        let config = get_config();
        let mut headers = HeaderMap::new();
        headers.insert("Tus-Resumable", HeaderValue::from_static(TUS_RESUMABLE));
        headers.insert("Authorization", HeaderValue::from_str(&config.token).unwrap());
        headers.insert("mac-identifier", HeaderValue::from_static("media-web"));
        headers.insert("mac-organization", HeaderValue::from_static("3"));

        headers
    }

    pub fn get_file_metadata(file_path: &Path) -> Result<String> {
        let filename = file_path.file_name()
            .ok_or(TusError::InvalidFile("Can't be read filename".to_string()))?
            .to_str()
            .ok_or(TusError::InvalidFile("The file name 'to_str' failed".to_string()))?;
        let encoded_filename = BASE64_STANDARD.encode(filename);

        Ok(format!("filename {}", encoded_filename))
    }

    pub fn parse_offset_header(status: u16, headers: &HeaderMap) -> Result<u64> {
        match headers.get("Upload-Offset") {
            Some(value) => {
                let offset =  value
                    .to_str()
                    .map_err(|err| TusError::HeaderParseError {
                        header_name: "Upload-Offset".to_string(),
                        message: err.to_string(),
                    })?
                    .parse::<u64>()
                    .map_err(|err| TusError::HeaderParseError {
                        header_name: "Upload-Offset".to_string(),
                        message: err.to_string(),
                    })?;
                
                Ok(offset)
            },
            None => Err(TusError::server_error(status, "No 'upload-offset' header in response"))
        }
    }
}
