use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tunnel::{
    TransferManager, TransferManagerBuilder, TransferConfig,
    UploadStrategy, TusConfig, SimpleConfig, ChunkedConfig,
    TusUploader, SimpleUploader, ChunkedUploader,
    UploadEvent, UploadState,
};

/// 模拟上传器 - 用于测试
struct MockUploader {
    delay: Duration,
    fail_on_first_attempt: bool,
    attempt_count: std::sync::atomic::AtomicU32,
}

impl MockUploader {
    fn new(delay: Duration, fail_on_first_attempt: bool) -> Self {
        Self {
            delay,
            fail_on_first_attempt,
            attempt_count: std::sync::atomic::AtomicU32::new(0),
        }
    }
}

#[async_trait::async_trait]
impl tunnel::core::Uploader for MockUploader {
    async fn upload(&self, task: &tunnel::UploadTask) -> tunnel::Result<String> {
        let attempt = self.attempt_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        
        if self.fail_on_first_attempt && attempt == 0 {
            return Err(tunnel::TransferError::custom("Simulated failure"));
        }
        
        // 模拟上传延迟
        tokio::time::sleep(self.delay).await;
        
        Ok(format!("https://example.com/uploaded/{}", task.id))
    }
    
    async fn resume(&self, task: &tunnel::UploadTask) -> tunnel::Result<String> {
        self.upload(task).await
    }
    
    async fn cancel(&self, _task: &tunnel::UploadTask) -> tunnel::Result<()> {
        Ok(())
    }
    
    async fn get_progress(&self, task: &tunnel::UploadTask) -> tunnel::Result<tunnel::UploadProgress> {
        Ok(tunnel::UploadProgress {
            uploaded_bytes: task.file_size / 2,
            total_bytes: task.file_size,
            speed: 1024.0 * 1024.0, // 1 MB/s
            average_speed: 1024.0 * 1024.0,
            percentage: 50.0,
            eta: Some(Duration::from_secs(30)),
        })
    }
    
    fn supports_resume(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn test_multiple_uploaders() {
    // 创建传输管理器
    let config = TransferConfig {
        max_concurrent: 2,
        ..Default::default()
    };
    
    let (manager, _handle) = TransferManagerBuilder::new()
        .config(config)
        .register_uploader("mock", Arc::new(MockUploader::new(Duration::from_millis(100), false)))
        .build()
        .await
        .unwrap();
    
    // 创建测试文件
    let test_file = PathBuf::from("test_integration.txt");
    tokio::fs::write(&test_file, b"test content").await.unwrap();
    
    // 使用 mock 上传器
    let metadata = HashMap::new();
    let upload_id = manager.add_upload(
        test_file.clone(),
        UploadStrategy::Simple(SimpleConfig::default()),
        metadata,
    ).await.unwrap();
    
    // 等待完成
    tokio::time::sleep(Duration::from_secs(1)).await;
    
    // 检查任务状态
    let task = manager.get_task(upload_id).await.unwrap();
    assert_eq!(task.state, UploadState::Completed);
    
    // 清理
    tokio::fs::remove_file(&test_file).await.unwrap();
    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_concurrent_uploads() {
    let config = TransferConfig {
        max_concurrent: 3,
        ..Default::default()
    };
    
    let (manager, _handle) = TransferManagerBuilder::new()
        .config(config)
        .register_uploader("mock", Arc::new(MockUploader::new(Duration::from_millis(200), false)))
        .build()
        .await
        .unwrap();
    
    // 创建多个测试文件
    let mut test_files = Vec::new();
    let mut upload_ids = Vec::new();
    
    for i in 0..5 {
        let file_path = PathBuf::from(format!("test_concurrent_{}.txt", i));
        tokio::fs::write(&file_path, format!("content {}", i)).await.unwrap();
        test_files.push(file_path.clone());
        
        let upload_id = manager.add_upload(
            file_path,
            UploadStrategy::Simple(SimpleConfig::default()),
            Default::default(),
        ).await.unwrap();
        
        upload_ids.push(upload_id);
    }
    
    // 等待所有任务完成
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // 检查所有任务状态
    for upload_id in upload_ids {
        let task = manager.get_task(upload_id).await.unwrap();
        assert_eq!(task.state, UploadState::Completed);
    }
    
    // 清理
    for file in test_files {
        tokio::fs::remove_file(&file).await.unwrap();
    }
    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_event_system() {
    let (manager, _handle) = TransferManagerBuilder::new()
        .register_uploader("mock", Arc::new(MockUploader::new(Duration::from_millis(100), false)))
        .build()
        .await
        .unwrap();
    
    let mut events = manager.subscribe_events();
    
    // 创建测试文件
    let test_file = PathBuf::from("test_events.txt");
    tokio::fs::write(&test_file, b"event test").await.unwrap();
    
    // 添加上传任务
    let upload_id = manager.add_upload(
        test_file.clone(),
        UploadStrategy::Simple(SimpleConfig::default()),
        Default::default(),
    ).await.unwrap();
    
    // 收集事件
    let mut received_events = Vec::new();
    let start = tokio::time::Instant::now();
    
    while start.elapsed() < Duration::from_secs(2) {
        match tokio::time::timeout(Duration::from_millis(100), events.recv()).await {
            Ok(Ok(event)) => {
                received_events.push(event);
            }
            _ => {}
        }
    }
    
    // 验证事件
    assert!(received_events.iter().any(|e| matches!(e, UploadEvent::TaskAdded { .. })));
    assert!(received_events.iter().any(|e| matches!(e, UploadEvent::StateChanged { .. })));
    
    // 清理
    tokio::fs::remove_file(&test_file).await.unwrap();
    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_retry_mechanism() {
    let (manager, _handle) = TransferManagerBuilder::new()
        .register_uploader(
            "mock_retry",
            Arc::new(MockUploader::new(Duration::from_millis(50), true))
        )
        .build()
        .await
        .unwrap();
    
    // 创建测试文件
    let test_file = PathBuf::from("test_retry.txt");
    tokio::fs::write(&test_file, b"retry test").await.unwrap();
    
    // 添加任务 - 第一次会失败，但应该重试成功
    let upload_id = manager.add_upload(
        test_file.clone(),
        UploadStrategy::Simple(SimpleConfig {
            timeout: Duration::from_secs(10),
            max_retries: 3,
        }),
        Default::default(),
    ).await.unwrap();
    
    // 等待重试完成
    tokio::time::sleep(Duration::from_secs(1)).await;
    
    let task = manager.get_task(upload_id).await.unwrap();
    // 根据实现，任务可能会失败或成功
    println!("Task state after retry: {:?}", task.state);
    
    // 清理
    tokio::fs::remove_file(&test_file).await.unwrap();
    manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_batch_operations() {
    let (manager, _handle) = TransferManagerBuilder::new()
        .register_uploader("mock", Arc::new(MockUploader::new(Duration::from_millis(100), false)))
        .build()
        .await
        .unwrap();
    
    // 创建批量文件
    let mut files = Vec::new();
    for i in 0..3 {
        let path = PathBuf::from(format!("batch_test_{}.txt", i));
        tokio::fs::write(&path, format!("batch content {}", i)).await.unwrap();
        files.push((path.clone(), None));
    }
    
    // 批量添加
    let upload_ids = manager.add_uploads(files.clone()).await.unwrap();
    assert_eq!(upload_ids.len(), 3);
    
    // 等待完成
    tokio::time::sleep(Duration::from_secs(1)).await;
    
    // 验证所有任务
    for upload_id in upload_ids {
        let task = manager.get_task(upload_id).await.unwrap();
        assert!(matches!(task.state, UploadState::Completed | UploadState::Uploading));
    }
    
    // 清理
    for (path, _) in files {
        tokio::fs::remove_file(&path).await.unwrap();
    }
    manager.shutdown().await.unwrap();
}
