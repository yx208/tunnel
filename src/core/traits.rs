use async_trait::async_trait;
use std::collections::HashMap;
use std::path::Path;
use super::types::{UploadTask, UploadProgress, UploadId};
use super::errors::Result;

/// 核心上传器 trait - 所有上传实现都必须实现此接口
#[async_trait]
pub trait Uploader: Send + Sync {
    /// 开始上传任务
    async fn upload(&self, task: &UploadTask) -> Result<String>;
    
    /// 恢复上传任务
    async fn resume(&self, task: &UploadTask) -> Result<String>;
    
    /// 取消上传任务
    async fn cancel(&self, task: &UploadTask) -> Result<()>;
    
    /// 获取上传进度
    async fn get_progress(&self, task: &UploadTask) -> Result<UploadProgress>;
    
    /// 检查是否支持断点续传
    fn supports_resume(&self) -> bool {
        false
    }
    
    /// 获取推荐的分块大小
    fn recommended_chunk_size(&self) -> usize {
        5 * 1024 * 1024 // 默认 5MB
    }
}

/// 上传器工厂 trait
pub trait UploaderFactory: Send + Sync {
    /// 创建上传器实例
    fn create(&self) -> Box<dyn Uploader>;
    
    /// 获取上传器名称
    fn name(&self) -> &str;
}

/// 进度回调 trait
#[async_trait]
pub trait ProgressCallback: Send + Sync {
    /// 进度更新回调
    async fn on_progress(&self, upload_id: UploadId, progress: &UploadProgress);
    
    /// 状态变更回调
    async fn on_state_change(&self, upload_id: UploadId, old_state: &str, new_state: &str);
    
    /// 上传完成回调
    async fn on_completed(&self, upload_id: UploadId, url: &str);
    
    /// 上传失败回调
    async fn on_failed(&self, upload_id: UploadId, error: &str);
}

/// 请求拦截器 trait
#[async_trait]
pub trait RequestInterceptor: Send + Sync {
    /// 请求前拦截
    async fn before_request(&self, request: &mut reqwest::Request) -> Result<()>;
    
    /// 响应后拦截
    async fn after_response(&self, response: &reqwest::Response) -> Result<()>;
}

/// 存储适配器 trait - 用于保存和恢复上传状态
#[async_trait]
pub trait StorageAdapter: Send + Sync {
    /// 保存上传任务状态
    async fn save_task(&self, task: &UploadTask) -> Result<()>;
    
    /// 加载上传任务状态
    async fn load_task(&self, upload_id: &UploadId) -> Result<Option<UploadTask>>;
    
    /// 删除上传任务状态
    async fn delete_task(&self, upload_id: &UploadId) -> Result<()>;
    
    /// 列出所有任务
    async fn list_tasks(&self) -> Result<Vec<UploadTask>>;
}
