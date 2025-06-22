use async_trait::async_trait;
use reqwest::{Client, Body};
use std::collections::HashMap;
use tokio::fs::File;
use tokio_util::io::ReaderStream;
use crate::core::{Uploader, UploadTask, UploadProgress, Result, TransferError, SimpleConfig};

/// 简单 HTTP 上传器
pub struct SimpleUploader {
    client: Client,
    endpoint: String,
    config: SimpleConfig,
}

impl SimpleUploader {
    pub fn new(endpoint: &str, config: SimpleConfig) -> Self {
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .unwrap();
        
        Self {
            client,
            endpoint: endpoint.to_string(),
            config,
        }
    }
}

#[async_trait]
impl Uploader for SimpleUploader {
    async fn upload(&self, task: &UploadTask) -> Result<String> {
        // 打开文件
        let file = File::open(&task.file_path).await?;
        let stream = ReaderStream::new(file);
        let body = Body::wrap_stream(stream);
        
        // 构建请求
        let mut request = self.client
            .post(&self.endpoint)
            .body(body);
        
        // 添加元数据作为表单字段或头部
        for (key, value) in &task.metadata {
            request = request.header(format!("X-Meta-{}", key), value);
        }
        
        // 发送请求
        let response = request
            .send()
            .await
            .map_err(|e| TransferError::Http(e))?;
        
        // 检查响应状态
        if !response.status().is_success() {
            return Err(TransferError::server_error(
                response.status().as_u16(),
                format!("Upload failed with status {}", response.status()),
            ));
        }
        
        // 返回响应中的 URL 或使用默认值
        let upload_url = response
            .headers()
            .get("Location")
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .unwrap_or_else(|| self.endpoint.clone());
        
        Ok(upload_url)
    }
    
    async fn resume(&self, _task: &UploadTask) -> Result<String> {
        Err(TransferError::unsupported("Simple uploader does not support resume"))
    }
    
    async fn cancel(&self, _task: &UploadTask) -> Result<()> {
        // 简单上传无法取消已开始的上传
        Ok(())
    }
    
    async fn get_progress(&self, task: &UploadTask) -> Result<UploadProgress> {
        // 简单上传无法获取实时进度
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
        false
    }
}

/// 带进度跟踪的简单上传器
pub struct SimpleUploaderWithProgress {
    inner: SimpleUploader,
    progress_sender: tokio::sync::mpsc::Sender<(crate::core::UploadId, u64)>,
}

impl SimpleUploaderWithProgress {
    pub fn new(
        endpoint: &str, 
        config: SimpleConfig,
        progress_sender: tokio::sync::mpsc::Sender<(crate::core::UploadId, u64)>,
    ) -> Self {
        Self {
            inner: SimpleUploader::new(endpoint, config),
            progress_sender,
        }
    }
}

#[async_trait]
impl Uploader for SimpleUploaderWithProgress {
    async fn upload(&self, task: &UploadTask) -> Result<String> {
        // TODO: 实现带进度跟踪的上传
        // 需要自定义 Stream 包装器来跟踪读取的字节数
        self.inner.upload(task).await
    }
    
    async fn resume(&self, task: &UploadTask) -> Result<String> {
        self.inner.resume(task).await
    }
    
    async fn cancel(&self, task: &UploadTask) -> Result<()> {
        self.inner.cancel(task).await
    }
    
    async fn get_progress(&self, task: &UploadTask) -> Result<UploadProgress> {
        self.inner.get_progress(task).await
    }
    
    fn supports_resume(&self) -> bool {
        self.inner.supports_resume()
    }
}
