use std::collections::HashMap;
use std::fs::File;
use std::io::SeekFrom;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use url::Url;
use anyhow::Context;
use reqwest::header::{HeaderValue, HeaderName, HeaderMap};
use reqwest::{Client, Request, Response, StatusCode};
use tokio::fs::File as TokioFile;
use tokio::io::AsyncSeekExt;
use tokio_util::io::ReaderStream;
use base64::Engine;
use crate::config::get_config;
use super::errors::{Result, TusError};
use super::progress::{ProgressCallback, ProgressInfo, ProgressStream, ProgressTracker};
use super::constants::TUS_RESUMABLE;

/// 请求钩子 trait
pub trait RequestHook: Sync + Send {
    fn before_request(&self, request: &mut Request) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UploadStrategy {
    /// Not support
    Chunked,

    /// Default strategy
    Streaming
}

#[derive(Clone)]
pub struct TusClient {
    pub client: Client,
    pub endpoint: String,
    pub buffer_size: usize,
    pub strategy: UploadStrategy,
    pub speed_limit: Option<u64>,
    pub headers: HashMap<String, String>,
    request_hook: Option<Arc<dyn RequestHook>>,
}

impl TusClient {
    pub fn new(endpoint: &str, buffer_size: usize) -> Self {
        Self {
            client: Client::new(),
            buffer_size,
            endpoint: endpoint.to_string(),
            strategy: UploadStrategy::Streaming,
            speed_limit: None,
            headers: HashMap::new(),
            request_hook: None,
        }
    }

    pub fn create_headers() -> HeaderMap {
        let config = get_config();
        let mut headers = HeaderMap::new();
        headers.insert("Tus-Resumable", HeaderValue::from_static(TUS_RESUMABLE));
        headers.insert("Authorization", HeaderValue::from_str(&config.token).unwrap());
        headers.insert("mac-identifier", HeaderValue::from_static("media-web"));
        headers.insert("mac-organization", HeaderValue::from_static("3"));

        headers
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

    pub fn with_hook(mut self, request_hook: impl RequestHook + 'static) -> Self {
        self.request_hook = Some(Arc::new(request_hook));
        self
    }

    /// 执行请求
    async fn execute(&self, mut request: Request) -> Result<Response> {
        if let Some(hook) = &self.request_hook {
            hook.before_request(&mut request)?;
        }

        Ok(self.client.execute(request).await?)
    }

    pub async fn create_upload(&self, file_size: u64, metadata: Option<HashMap<String, String>>) -> Result<String> {
        let mut request = self.client
            .post(&self.endpoint)
            .header("Tus-Resumable", "1.0.0")
            .header("Upload-Length", file_size);

        // Insert metadata header
        if let Some(meta) = metadata {
            for (k, v) in meta {
                request = request.header(
                    HeaderName::from_str(&k)?,
                    HeaderValue::from_str(&v)?
                );
            }
        }
        
        let request = request.build()?;
        let response = self.execute(request).await?;

        if response.status() != StatusCode::CREATED {
            return Err(TusError::server_error(response.status().as_u16(), "Failed to create upload"));
        }

        let location = match response.headers().get("location") {
            Some(loc) => loc.to_str()?.to_string(),
            None => {
                return Err(TusError::server_error(
                    response.status().as_u16(),
                    "Not 'location' header in response",
                ));
            }
        };

        if location.starts_with("http") {
            Ok(location)
        } else {
            let url = Url::parse(&self.endpoint)
                .map_err(|_e| TusError::ParamError(format!("Invalid url: {:?}", self.endpoint)))?;
            let origin = url.origin().ascii_serialization();

            Ok(format!("{}{}", origin, location))
        }
    }

    pub async fn get_upload_offset(&self, upload_url: &str) -> Result<u64> {
        let request = self.client
            .head(upload_url)
            .header("Tus-Resumable", "1.0.0")
            .build()?;
        let response = self.execute(request).await?;

        let status = response.status();
        if response.status() != StatusCode::OK && response.status() != StatusCode::NO_CONTENT {
            return Err(TusError::server_error(response.status().as_u16(), "Failed to get upload offset"));
        }

        let offset = TusClient::parse_offset_header(status.as_u16(), response.headers())?;

        Ok(offset)
    }

    pub async fn upload_file_streaming(
        &self,
        upload_url: &str,
        file_path: &str,
        file_size: u64,
        progress_callback: Option<ProgressCallback>
    ) -> Result<String> {
        let offset = self.get_upload_offset(upload_url).await?;

        if offset > file_size {
            if let Some(callback) = progress_callback {
                callback(ProgressInfo {
                    bytes_uploaded: file_size,
                    total_bytes: file_size,
                    instant_speed: 0.0,
                    average_speed: 0.0,
                    eta: None,
                    percentage: 0.0,
                });
            }

            return Ok(upload_url.to_string());
        }

        let file = TokioFile::open(file_path).await
            .with_context(|| format!("Failed to open file: {}", file_path))?;

        let mut file = file.try_clone().await?;
        file.seek(SeekFrom::Start(offset)).await?;

        let reader_stream = ReaderStream::with_capacity(file, self.buffer_size);

        let body = if let Some(callback) = progress_callback {
            let tracker = Arc::new(
                ProgressTracker::new(file_size, offset)
                    .with_callback(callback)
                    .with_update_interval(Duration::from_secs(1))
            );

            // 立即发送初始进度
            if offset > 0 {
                let initial_info = ProgressInfo {
                    bytes_uploaded: offset,
                    total_bytes: file_size,
                    instant_speed: 0.0,
                    average_speed: 0.0,
                    eta: None,
                    percentage: (offset as f64 / file_size as f64) * 100.0,
                };
                (tracker.callback.as_ref().unwrap())(initial_info);
            }

            let progress_stream = ProgressStream::new(reader_stream, tracker);
            reqwest::Body::wrap_stream(progress_stream)
        } else {
            reqwest::Body::wrap_stream(reader_stream)
        };

        let remaining_size = file_size - offset;
        let request = self.client
            .patch(upload_url)
            .body(body)
            .header("Tus-Resumable", "1.0.0")
            .header("Content-Type", "application/offset+octet-stream")
            .header("Content-Length", remaining_size)
            .header("Upload-Offset", offset)
            .build()?;
        
        let response = self.execute(request).await?;

        // 验证响应
        let status = response.status();
        if status != StatusCode::NO_CONTENT {
            return Err(TusError::server_error(
                status.as_u16(),
                format!("Upload failed with status {}", status),
            ));
        }

        // 验证最终偏移量
        let final_offset = TusClient::parse_offset_header(status.as_u16(), response.headers())?;
        if final_offset != file_size {
            return Err(TusError::UploadIncomplete {
                expected: file_size,
                actual: final_offset,
            });
        }

        Ok(upload_url.to_string())
    }

    pub async fn upload_file(
        &self,
        file_path: &str,
        metadata: Option<HashMap<String, String>>,
        callback: Option<ProgressCallback>
    ) -> Result<String>
    {
        let file_size = {
            let path = Path::new(file_path);
            let mut file =
                File::open(path).with_context(|| format!("Failed to open file: {}", file_path))?;
            let file_size = file
                .metadata()
                .with_context(|| format!("Failed to get metadata for file: {}", file_path))?
                .len();

            file_size
        };

        // Create TUS upload URL
        let upload_url = self.create_upload(file_size, metadata).await?;

        match self.strategy {
            UploadStrategy::Streaming => {
                self.upload_file_streaming(&upload_url, &file_path, file_size, callback).await?;
            }
            _ => {}
        };

        Ok(upload_url)
    }

    pub async fn cancel_upload(&self, upload_url: &str) -> Result<()> {
        let headers = TusClient::create_headers();

        let response = self
            .client
            .delete(upload_url)
            .headers(headers)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() && status != StatusCode::NOT_FOUND {
            return Err(TusError::server_error(status.as_u16(), "Failed to cancel upload"));
        }

        Ok(())
    }
}
