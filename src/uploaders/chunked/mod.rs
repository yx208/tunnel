use async_trait::async_trait;
use futures::future::join_all;
use reqwest::Client;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::{Mutex, Semaphore};
use crate::core::{Uploader, UploadTask, UploadProgress, Result, TransferError, ChunkedConfig};

/// 分片信息
#[derive(Debug, Clone)]
struct ChunkInfo {
    index: usize,
    offset: u64,
    size: usize,
    uploaded: bool,
}

/// 分片上传器
pub struct ChunkedUploader {
    client: Client,
    endpoint: String,
    config: ChunkedConfig,
}

impl ChunkedUploader {
    pub fn new(endpoint: &str, config: ChunkedConfig) -> Self {
        let client = Client::new();
        
        Self {
            client,
            endpoint: endpoint.to_string(),
            config,
        }
    }
    
    /// 计算分片信息
    fn calculate_chunks(&self, file_size: u64) -> Vec<ChunkInfo> {
        let chunk_size = self.config.chunk_size as u64;
        let mut chunks = Vec::new();
        let mut offset = 0;
        let mut index = 0;
        
        while offset < file_size {
            let size = std::cmp::min(chunk_size, file_size - offset) as usize;
            chunks.push(ChunkInfo {
                index,
                offset,
                size,
                uploaded: false,
            });
            offset += size as u64;
            index += 1;
        }
        
        chunks
    }
    
    /// 初始化分片上传
    async fn init_multipart_upload(&self, task: &UploadTask) -> Result<String> {
        let response = self.client
            .post(format!("{}/init", self.endpoint))
            .json(&serde_json::json!({
                "filename": task.file_path.file_name().unwrap().to_str().unwrap(),
                "size": task.file_size,
                "metadata": task.metadata,
            }))
            .send()
            .await?;
        
        if !response.status().is_success() {
            return Err(TransferError::server_error(
                response.status().as_u16(),
                "Failed to initialize multipart upload",
            ));
        }
        
        let result: serde_json::Value = response.json().await?;
        let upload_id = result["upload_id"]
            .as_str()
            .ok_or_else(|| TransferError::custom("No upload_id in response"))?;
        
        Ok(upload_id.to_string())
    }
    
    /// 上传单个分片
    async fn upload_chunk(
        &self,
        file_path: &std::path::Path,
        upload_id: &str,
        chunk: &ChunkInfo,
    ) -> Result<()> {
        // 读取分片数据
        let mut file = File::open(file_path).await?;
        file.seek(std::io::SeekFrom::Start(chunk.offset)).await?;
        
        let mut buffer = vec![0u8; chunk.size];
        file.read_exact(&mut buffer).await?;
        
        // 上传分片
        let response = self.client
            .put(format!("{}/chunk", self.endpoint))
            .header("X-Upload-Id", upload_id)
            .header("X-Chunk-Index", chunk.index.to_string())
            .header("X-Chunk-Offset", chunk.offset.to_string())
            .body(buffer)
            .send()
            .await?;
        
        if !response.status().is_success() {
            return Err(TransferError::server_error(
                response.status().as_u16(),
                format!("Failed to upload chunk {}", chunk.index),
            ));
        }
        
        Ok(())
    }
    
    /// 完成分片上传
    async fn complete_multipart_upload(&self, upload_id: &str, chunks: &[ChunkInfo]) -> Result<String> {
        let chunk_indices: Vec<usize> = chunks.iter().map(|c| c.index).collect();
        
        let response = self.client
            .post(format!("{}/complete", self.endpoint))
            .json(&serde_json::json!({
                "upload_id": upload_id,
                "chunks": chunk_indices,
            }))
            .send()
            .await?;
        
        if !response.status().is_success() {
            return Err(TransferError::server_error(
                response.status().as_u16(),
                "Failed to complete multipart upload",
            ));
        }
        
        let result: serde_json::Value = response.json().await?;
        let url = result["url"]
            .as_str()
            .ok_or_else(|| TransferError::custom("No url in response"))?;
        
        Ok(url.to_string())
    }
}

#[async_trait]
impl Uploader for ChunkedUploader {
    async fn upload(&self, task: &UploadTask) -> Result<String> {
        // 初始化分片上传
        let upload_id = self.init_multipart_upload(task).await?;
        
        // 计算分片
        let chunks = self.calculate_chunks(task.file_size);
        let total_chunks = chunks.len();
        
        // 创建信号量限制并发数
        let semaphore = Arc::new(Semaphore::new(self.config.concurrent_chunks));
        let uploaded_count = Arc::new(Mutex::new(0usize));
        
        // 并发上传分片
        let upload_futures = chunks.iter().map(|chunk| {
            let client = self.client.clone();
            let endpoint = self.endpoint.clone();
            let file_path = task.file_path.clone();
            let upload_id = upload_id.clone();
            let chunk = chunk.clone();
            let semaphore = semaphore.clone();
            let uploaded_count = uploaded_count.clone();
            
            async move {
                let _permit = semaphore.acquire().await.unwrap();
                
                // 上传分片
                self.upload_chunk(&file_path, &upload_id, &chunk).await?;
                
                // 更新计数
                let mut count = uploaded_count.lock().await;
                *count += 1;
                
                Ok::<(), TransferError>(())
            }
        });
        
        // 等待所有分片上传完成
        let results = join_all(upload_futures).await;
        
        // 检查是否有失败的分片
        for result in results {
            result?;
        }
        
        // 完成分片上传
        let url = self.complete_multipart_upload(&upload_id, &chunks).await?;
        
        Ok(url)
    }
    
    async fn resume(&self, task: &UploadTask) -> Result<String> {
        // TODO: 实现断点续传
        // 需要从服务器查询已上传的分片，然后只上传未完成的分片
        Err(TransferError::unsupported("Chunked resume not implemented yet"))
    }
    
    async fn cancel(&self, task: &UploadTask) -> Result<()> {
        if let Some(upload_url) = &task.upload_url {
            // 从 URL 中提取 upload_id
            // TODO: 实现取消逻辑
        }
        Ok(())
    }
    
    async fn get_progress(&self, task: &UploadTask) -> Result<UploadProgress> {
        // TODO: 实现进度查询
        Ok(UploadProgress {
            uploaded_bytes: 0,
            total_bytes: task.file_size,
            speed: 0.0,
            average_speed: 0.0,
            percentage: 0.0,
            eta: None,
        })
    }
    
    fn supports_resume(&self) -> bool {
        true // 理论上支持，但需要实现
    }
    
    fn recommended_chunk_size(&self) -> usize {
        self.config.chunk_size
    }
}
