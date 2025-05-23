use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;
use anyhow::Context;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::{Body, Client, StatusCode};
use reqwest::header::HeaderValue;
use url::Url;
use tokio::{
    fs::File as TokioFile,
};

use tokio::io::AsyncSeekExt;
use tokio_util::codec::{BytesCodec, FramedRead};
use super::types::{TusClient, UploadStrategy};
use super::errors::{Result, TusError};

impl TusClient {
    pub fn new(endpoint: &str, chunk_size: usize) -> Self {
        Self {
            client: Client::new(),
            chunk_size,
            endpoint: endpoint.to_string(),
            strategy: UploadStrategy::Chunked,
            speed_limit: None
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

    async fn create_upload(&self, file_size: u64, metadata: Option<&str>) -> Result<String> {
        let mut headers = TusClient::create_headers();
        headers.insert("Upload-Length", HeaderValue::from_str(&file_size.to_string())?);

        if let Some(meta) = metadata {
            headers.insert("Upload-Metadata", HeaderValue::from_str(meta)?);
        }

        let response = self
            .client
            .post(&self.endpoint)
            .headers(headers)
            .send()
            .await?;

        if response.status() != StatusCode::CREATED {
            return Err(TusError::UnexpectedStatusCode(response.status()));
        }

        let location = match response.headers().get("location") {
            Some(loc) => loc.to_str()?.to_string(),
            None => return Err(TusError::ProtocolError("Not 'location' header in response".to_string())),
        };

        if location.starts_with("http") {
            Ok(location)
        } else {
            let url = Url::parse(&self.endpoint)?;
            let origin = url.origin().ascii_serialization();

            Ok(format!("{}{}", origin, location))
        }
    }

    async fn get_upload_offset(&self, upload_url: &str) -> Result<u64> {
        let headers = TusClient::create_headers();

        let response = self
            .client
            .head(upload_url)
            .headers(headers)
            .send()
            .await?;

        if response.status() != StatusCode::OK && response.status() != StatusCode::NO_CONTENT {
            return Err(TusError::UnexpectedStatusCode(response.status()));
        }

        let offset = TusClient::parse_offset_header(response.headers())?;

        Ok(offset)
    }

    async fn upload_chunk(&self, upload_url: &str, file: &mut File, offset: u64) -> Result<u64> {
        file.seek(SeekFrom::Start(offset))?;
        let mut buffer = vec![0; self.chunk_size];
        let bytes_read = file.read(&mut buffer)?;

        if bytes_read == 0 {
            return Ok(offset)
        }

        buffer.truncate(bytes_read);

        let mut headers = TusClient::create_headers();
        headers.insert("Upload-Offset", HeaderValue::from_str(&offset.to_string())?);
        headers.insert("Content-Type", HeaderValue::from_static("application/offset+octet-stream"));

        let response = self
            .client
            .patch(upload_url)
            .headers(headers)
            .body(buffer)
            .send()
            .await?;

        if response.status() != StatusCode::NO_CONTENT {
            return Err(TusError::UnexpectedStatusCode(response.status()));
        }

        let next_offset = TusClient::parse_offset_header(response.headers())?;

        Ok(next_offset)
    }

    async fn upload_file_chunked(&self, upload_url: &str, file: &mut File, file_size: u64) -> Result<()> {
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
                println!("Uploaded (chunked): {}/{} bytes ({}%)",
                         offset,
                         file_size,
                         (offset as f64 / file_size as f64 * 100.0) as u64
                );
            }

            // If speed limit
            if let Some(max_bytes_per_sec) = self.speed_limit {
                // 开始到现在过去了多久（秒）
                let current_elapsed = std::time::Instant::now().duration_since(start_time).as_secs_f64();
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

    async fn create_upload_stream(&self, file_path: &str, offset: u64)
        -> Result<FramedRead<TokioFile, BytesCodec>>
    {
        let mut file = TokioFile::open(file_path)
            .await
            .with_context(|| format!("Failed to open file: {}", file_path))?;

        file.seek(SeekFrom::Start(offset))
            .await
            .with_context(|| "Failed to seek to offset")?;

        let codec = BytesCodec::new();
        let stream = FramedRead::new(file, codec);

        Ok(stream)
    }

    async fn upload_file_single_request(&self, upload_url: &str, file_path: &str, file_size: u64) -> Result<()> {
        let offset = self.get_upload_offset(upload_url).await?;

        if offset > file_size {
            return Ok(());
        }

        let mut file = TokioFile::open(file_path)
            .await
            .with_context(|| format!("Failed to open file: {}", file_path))?;

        file.seek(SeekFrom::Start(offset))
            .await
            .with_context(|| "Failed to seek to offset")?;

        let stream = FramedRead::new(file, BytesCodec::new());
        let body = Body::wrap_stream(stream);

        let mut headers = TusClient::create_headers();
        headers.insert("Upload-Offset", HeaderValue::from_str(&offset.to_string())?);
        headers.insert("Content-Type", HeaderValue::from_static("application/offset+octet-stream"));

        let response = self
            .client
            .patch(upload_url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .context("Failed to send file upload request")?;

        let status = response.status();
        if status != StatusCode::NO_CONTENT {
            return Err(TusError::server_error(
                status.as_u16(),
                format!("Failed to upload file: unexpected status {}", status),
            ));
        }

        Ok(())
    }

     pub async fn upload_file(&self, file_path: &str, metadata: Option<&str>) -> Result<String> {
         let path = Path::new(file_path);
         let mut file = File::open(path)
             .with_context(|| format!("Failed to open file: {}", file_path))?;

         let file_size = file.metadata()
             .with_context(|| format!("Failed to get metadata for file: {}", file_path))?
             .len();

         let upload_url = self.create_upload(file_size, metadata).await?;

         match self.strategy {
             UploadStrategy::Chunked => {
                self.upload_file_chunked(&upload_url, &mut file, file_size).await?;
             }
             UploadStrategy::SingleRequest => {
                 drop(file);
                 self.upload_file_single_request(&upload_url, &file_path, file_size).await?;
             }
         };

         Ok(upload_url)
     }
}
