use std::collections::HashMap;
use anyhow::Context;
use reqwest::header::{HeaderValue, HeaderName, HeaderMap};
use reqwest::{Client, StatusCode};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use url::Url;
use tokio::fs::File as TokioFile;
use tokio::io::AsyncSeekExt;
use tokio_util::io::ReaderStream;
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use crate::config::get_config;
use super::errors::{Result, TusError};
use super::progress::{ProgressCallback, ProgressInfo, ProgressStream, ProgressTracker};
use super::constants::TUS_RESUMABLE;

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

#[derive(Debug, Clone)]
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

impl TusClient {
    pub fn new(endpoint: &str, chunk_size: usize) -> Self {
        Self {
            client: Client::new(),
            chunk_size,
            endpoint: endpoint.to_string(),
            strategy: UploadStrategy::Chunked,
            speed_limit: None,
        }
    }

    pub fn with_strategy(endpoint: &str, chunk_size: usize, strategy: UploadStrategy) -> Self {
        Self {
            client: Client::new(),
            endpoint: endpoint.to_string(),
            chunk_size,
            strategy,
            speed_limit: None,
        }
    }

    pub async fn create_upload(&self, file_size: u64, metadata: Option<HashMap<String, String>>) -> Result<String> {
        let mut headers = TusClient::create_headers();
        headers.insert("Upload-Length", file_size.to_string().parse()?);

        if let Some(meta) = metadata {
            for (k, v) in meta {
                headers.insert(HeaderName::from_str(&k)?, v.parse()?);
            }
        }

        let response = self
            .client
            .post(&self.endpoint)
            .headers(headers)
            .send()
            .await?;

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
        let headers = TusClient::create_headers();

        let response = self.client.head(upload_url).headers(headers).send().await?;

        let status = response.status();
        if response.status() != StatusCode::OK && response.status() != StatusCode::NO_CONTENT {
            return Err(TusError::server_error(response.status().as_u16(), "Failed to get upload offset"));
        }

        let offset = TusClient::parse_offset_header(status.as_u16(), response.headers())?;

        Ok(offset)
    }

    async fn upload_chunk(&self, upload_url: &str, file: &mut File, offset: u64) -> Result<u64> {
        file.seek(SeekFrom::Start(offset))?;
        let mut buffer = vec![0; self.chunk_size];
        let bytes_read = file.read(&mut buffer)?;

        if bytes_read == 0 {
            return Ok(offset);
        }

        buffer.truncate(bytes_read);

        let mut headers = TusClient::create_headers();
        headers.insert("Upload-Offset", HeaderValue::from_str(&offset.to_string())?);
        headers.insert(
            "Content-Type",
            HeaderValue::from_static("application/offset+octet-stream"),
        );

        let response = self
            .client
            .patch(upload_url)
            .headers(headers)
            .body(buffer)
            .send()
            .await?;

        let status = response.status();
        if status != StatusCode::NO_CONTENT {
            return Err(TusError::server_error(status.as_u16(), "Failed to patch file"));
        }

        let next_offset = TusClient::parse_offset_header(status.as_u16(), response.headers())?;

        Ok(next_offset)
    }

    async fn upload_file_chunked(&self, upload_url: &str, file: &mut File, file_size: u64, ) -> Result<()> {
        // 总是等于 已上传大小 + ChunkSize
        let mut offset = self.get_upload_offset(upload_url).await?;

        let start_time = std::time::Instant::now();
        let mut last_progress_update = std::time::Instant::now();
        let mut bytes_since_last_update = 0;

        while offset < file_size {
            // 记录当前块上传前的偏移量
            let pre_chunk_offset = offset;

            // Upload
            offset = self.upload_chunk(upload_url, file, offset).await?;

            // 计算这次上传的字节数
            let bytes_uploaded_this_chunk = offset - pre_chunk_offset;
            bytes_since_last_update += bytes_uploaded_this_chunk;

            // 每秒更新一次进度
            let now = std::time::Instant::now();
            if now.duration_since(last_progress_update).as_secs() >= 1 {
                let elapsed_since_last = now.duration_since(last_progress_update).as_secs_f64();
                let current_speed = bytes_since_last_update as f64 / elapsed_since_last;

                let total_elapsed = now.duration_since(start_time).as_secs_f64();
                let avg_speed = offset as f64 / total_elapsed;

                println!(
                    "Uploaded (chunked): {}/{} bytes ({}%), Current speed: {:.2} MB/s, Avg speed: {:.2} MB/s",
                    offset,
                    file_size,
                    (offset as f64 / file_size as f64 * 100.0) as u64,
                    current_speed / (1024.0 * 1024.0),
                    avg_speed / (1024.0 * 1024.0)
                );

                last_progress_update = now;
                bytes_since_last_update = 0;
            } else {
                println!(
                    "Uploaded (chunked): {}/{} bytes ({}%)",
                    offset,
                    file_size,
                    (offset as f64 / file_size as f64 * 100.0) as u64
                );
            }

            // If speed limit
            if let Some(max_bytes_per_sec) = self.speed_limit {
                // 开始到现在过去了多久（秒）
                let current_elapsed = std::time::Instant::now()
                    .duration_since(start_time)
                    .as_secs_f64();
                // 按照当前过去的时间跟已上传的总字节，计算出按照目前网速每秒能上传的速度
                let current_rate = offset as f64 / current_elapsed;

                // 超过设定的阈值
                if current_rate > max_bytes_per_sec as f64 {
                    // 计算等待时间，已上传字节 / 预算每秒速度 = 应当需要的时间（秒）
                    let target_time = offset as f64 / max_bytes_per_sec as f64;
                    //  当前速率
                    let wait_time = target_time - current_rate;

                    if wait_time > 0.0 {
                        tokio::time::sleep(Duration::from_secs_f64(wait_time)).await;
                    }
                }
            }
        }

        let total_time = start_time.elapsed().as_secs_f64();
        if total_time > 0.0 {
            let avg_speed = file_size as f64 / total_time;
            println!(
                "Upload completed. Total: {} bytes, Time: {:.2}s, Avg speed: {:.2} MB/s",
                file_size,
                total_time,
                avg_speed / (1024.0 * 1024.0)
            );
        }

        Ok(())
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

        // 64KB
        let reader_stream = ReaderStream::with_capacity(file, 1024 * 1024);

        let body = if let Some(callback) = progress_callback {
            let tracker = Arc::new(
                ProgressTracker::new(file_size, offset)
                    .with_callback(callback)
                    .with_update_interval(Duration::from_millis(500))
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
        let mut headers = TusClient::create_headers();
        headers.insert("Content-Type", HeaderValue::from_static("application/offset+octet-stream"));
        headers.insert("Upload-Offset", HeaderValue::from_str(&offset.to_string())?);
        headers.insert("Content-Length", HeaderValue::from_str(&remaining_size.to_string())?);

        let response = self.client
            .patch(upload_url)
            .headers(headers)
            .body(body)
            .send()
            .await?;

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

    pub async fn upload_file(&self, file_path: &str, metadata: Option<HashMap<String, String>>) -> Result<String> {
        let path = Path::new(file_path);
        let mut file =
            File::open(path).with_context(|| format!("Failed to open file: {}", file_path))?;

        let file_size = file
            .metadata()
            .with_context(|| format!("Failed to get metadata for file: {}", file_path))?
            .len();

        let upload_url = self.create_upload(file_size, metadata).await?;

        match self.strategy {
            UploadStrategy::Chunked => {
                self.upload_file_chunked(&upload_url, &mut file, file_size)
                    .await?;
            }
            UploadStrategy::Streaming => {
                drop(file);

                let callback = Arc::new(|info: ProgressInfo| {
                    let eta_str = info.eta
                        .map(|d| format!("ETA: {}s", d.as_secs()))
                        .unwrap_or_default();

                    println!(
                        "{:.2} MB/s{}",
                        info.instant_speed,
                        eta_str
                    );
                });

                self.upload_file_streaming(&upload_url, &file_path, file_size, Some(callback)).await?;
            }
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
