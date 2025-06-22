use async_trait::async_trait;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::fs::File;
use crate::core::{Uploader, UploadTask, UploadProgress, Result, TransferError};
use crate::tus::TusClient;

/// Tus 协议上传器
pub struct TusUploader {
    client: Arc<TusClient>,
}

impl TusUploader {
    pub fn new(endpoint: &str, chunk_size: usize) -> Self {
        let client = TusClient::new(endpoint, chunk_size);
        Self {
            client: Arc::new(client),
        }
    }
    
    pub fn with_client(client: TusClient) -> Self {
        Self {
            client: Arc::new(client),
        }
    }
}

#[async_trait]
impl Uploader for TusUploader {
    async fn upload(&self, task: &UploadTask) -> Result<String> {
        // 创建上传会话
        let upload_url = self.client
            .create_upload(task.file_size, Some(task.metadata.clone()))
            .await
            .map_err(|e| TransferError::custom(e.to_string()))?;
        
        // 打开文件
        let file = File::open(&task.file_path).await?;
        let stream = tokio_util::io::ReaderStream::new(file);
        let body = reqwest::Body::wrap_stream(stream);
        
        // 执行上传
        let result = self.client
            .upload_file_streaming(&upload_url, task.file_size, 0, body)
            .await
            .map_err(|e| TransferError::custom(e.to_string()))?;
        
        Ok(result)
    }
    
    async fn resume(&self, task: &UploadTask) -> Result<String> {
        let upload_url = task.upload_url.as_ref()
            .ok_or_else(|| TransferError::invalid_parameter("No upload URL for resume"))?;
        
        // 获取已上传的偏移量
        let offset = self.client
            .get_upload_offset(upload_url)
            .await
            .map_err(|e| TransferError::custom(e.to_string()))?;
        
        // 打开文件并跳过已上传的部分
        let mut file = File::open(&task.file_path).await?;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        
        let stream = tokio_util::io::ReaderStream::new(file);
        let body = reqwest::Body::wrap_stream(stream);
        
        // 继续上传
        let result = self.client
            .upload_file_streaming(upload_url, task.file_size, offset, body)
            .await
            .map_err(|e| TransferError::custom(e.to_string()))?;
        
        Ok(result)
    }
    
    async fn cancel(&self, task: &UploadTask) -> Result<()> {
        if let Some(upload_url) = &task.upload_url {
            self.client
                .cancel_upload(upload_url)
                .await
                .map_err(|e| TransferError::custom(e.to_string()))?;
        }
        Ok(())
    }
    
    async fn get_progress(&self, task: &UploadTask) -> Result<UploadProgress> {
        let upload_url = task.upload_url.as_ref()
            .ok_or_else(|| TransferError::invalid_parameter("No upload URL"))?;
        
        let offset = self.client
            .get_upload_offset(upload_url)
            .await
            .map_err(|e| TransferError::custom(e.to_string()))?;
        
        let percentage = if task.file_size > 0 {
            (offset as f64 / task.file_size as f64) * 100.0
        } else {
            0.0
        };
        
        Ok(UploadProgress {
            uploaded_bytes: offset,
            total_bytes: task.file_size,
            speed: 0.0, // TODO: 计算实时速度
            average_speed: 0.0, // TODO: 计算平均速度
            percentage,
            eta: None, // TODO: 计算预计剩余时间
        })
    }
    
    fn supports_resume(&self) -> bool {
        true
    }
    
    fn recommended_chunk_size(&self) -> usize {
        self.client.buffer_size
    }
}
