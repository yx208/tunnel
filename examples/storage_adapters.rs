use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tunnel::core::{StorageAdapter, UploadTask, UploadId, Result, TransferError};

/// 内存存储适配器（用于测试）
pub struct MemoryStorageAdapter {
    tasks: Arc<RwLock<HashMap<UploadId, UploadTask>>>,
}

impl MemoryStorageAdapter {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl StorageAdapter for MemoryStorageAdapter {
    async fn save_task(&self, task: &UploadTask) -> Result<()> {
        let mut tasks = self.tasks.write().await;
        tasks.insert(task.id, task.clone());
        Ok(())
    }
    
    async fn load_task(&self, upload_id: &UploadId) -> Result<Option<UploadTask>> {
        let tasks = self.tasks.read().await;
        Ok(tasks.get(upload_id).cloned())
    }
    
    async fn delete_task(&self, upload_id: &UploadId) -> Result<()> {
        let mut tasks = self.tasks.write().await;
        tasks.remove(upload_id);
        Ok(())
    }
    
    async fn list_tasks(&self) -> Result<Vec<UploadTask>> {
        let tasks = self.tasks.read().await;
        Ok(tasks.values().cloned().collect())
    }
}

/// 文件系统存储适配器
pub struct FileStorageAdapter {
    state_file: PathBuf,
}

impl FileStorageAdapter {
    pub fn new(state_file: PathBuf) -> Self {
        Self { state_file }
    }
    
    async fn load_state(&self) -> Result<HashMap<UploadId, UploadTask>> {
        if !self.state_file.exists() {
            return Ok(HashMap::new());
        }
        
        let content = tokio::fs::read_to_string(&self.state_file)
            .await
            .map_err(|e| TransferError::Storage(format!("Failed to read state file: {}", e)))?;
        
        serde_json::from_str(&content)
            .map_err(|e| TransferError::Storage(format!("Failed to parse state file: {}", e)))
    }
    
    async fn save_state(&self, tasks: &HashMap<UploadId, UploadTask>) -> Result<()> {
        let content = serde_json::to_string_pretty(tasks)
            .map_err(|e| TransferError::Storage(format!("Failed to serialize state: {}", e)))?;
        
        tokio::fs::write(&self.state_file, content)
            .await
            .map_err(|e| TransferError::Storage(format!("Failed to write state file: {}", e)))?;
        
        Ok(())
    }
}

#[async_trait]
impl StorageAdapter for FileStorageAdapter {
    async fn save_task(&self, task: &UploadTask) -> Result<()> {
        let mut tasks = self.load_state().await?;
        tasks.insert(task.id, task.clone());
        self.save_state(&tasks).await
    }
    
    async fn load_task(&self, upload_id: &UploadId) -> Result<Option<UploadTask>> {
        let tasks = self.load_state().await?;
        Ok(tasks.get(upload_id).cloned())
    }
    
    async fn delete_task(&self, upload_id: &UploadId) -> Result<()> {
        let mut tasks = self.load_state().await?;
        tasks.remove(upload_id);
        self.save_state(&tasks).await
    }
    
    async fn list_tasks(&self) -> Result<Vec<UploadTask>> {
        let tasks = self.load_state().await?;
        Ok(tasks.into_values().collect())
    }
}

/// 带缓存的存储适配器
pub struct CachedStorageAdapter<T: StorageAdapter> {
    inner: T,
    cache: Arc<RwLock<HashMap<UploadId, UploadTask>>>,
}

impl<T: StorageAdapter> CachedStorageAdapter<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    pub async fn load_cache(&self) -> Result<()> {
        let tasks = self.inner.list_tasks().await?;
        let mut cache = self.cache.write().await;
        for task in tasks {
            cache.insert(task.id, task);
        }
        Ok(())
    }
}

#[async_trait]
impl<T: StorageAdapter + Send + Sync> StorageAdapter for CachedStorageAdapter<T> {
    async fn save_task(&self, task: &UploadTask) -> Result<()> {
        // 更新缓存
        {
            let mut cache = self.cache.write().await;
            cache.insert(task.id, task.clone());
        }
        
        // 保存到底层存储
        self.inner.save_task(task).await
    }
    
    async fn load_task(&self, upload_id: &UploadId) -> Result<Option<UploadTask>> {
        // 先从缓存查找
        {
            let cache = self.cache.read().await;
            if let Some(task) = cache.get(upload_id) {
                return Ok(Some(task.clone()));
            }
        }
        
        // 从底层存储加载
        if let Some(task) = self.inner.load_task(upload_id).await? {
            // 更新缓存
            let mut cache = self.cache.write().await;
            cache.insert(task.id, task.clone());
            Ok(Some(task))
        } else {
            Ok(None)
        }
    }
    
    async fn delete_task(&self, upload_id: &UploadId) -> Result<()> {
        // 从缓存删除
        {
            let mut cache = self.cache.write().await;
            cache.remove(upload_id);
        }
        
        // 从底层存储删除
        self.inner.delete_task(upload_id).await
    }
    
    async fn list_tasks(&self) -> Result<Vec<UploadTask>> {
        let cache = self.cache.read().await;
        Ok(cache.values().cloned().collect())
    }
}
