#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::fs;
    use crate::core::*;
    use crate::uploaders::*;

    // 创建测试文件
    async fn create_test_file(path: &str, size: usize) -> Result<PathBuf> {
        let path = PathBuf::from(path);
        let data = vec![0u8; size];
        fs::write(&path, data).await?;
        Ok(path)
    }

    // 清理测试文件
    async fn cleanup_test_file(path: &PathBuf) {
        let _ = fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn test_transfer_manager_creation() {
        let config = TransferConfig::default();
        let (manager, handle) = TransferManager::new(config);
        
        // 注册上传器
        let tus_uploader = Arc::new(TusUploader::new("http://localhost:1080", 1024 * 1024));
        manager.register_uploader("tus", tus_uploader).await.unwrap();
        
        // 关闭管理器
        manager.shutdown().await.unwrap();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_add_upload_task() {
        let config = TransferConfig::default();
        let (manager, handle) = TransferManager::new(config);
        
        // 创建测试文件
        let test_file = create_test_file("test_upload.bin", 1024).await.unwrap();
        
        // 添加上传任务
        let upload_id = manager.add_upload(
            test_file.clone(),
            UploadStrategy::Tus(TusConfig::default()),
            Default::default(),
        ).await.unwrap();
        
        // 获取任务信息
        let task = manager.get_task(upload_id).await;
        assert!(task.is_some());
        assert_eq!(task.unwrap().id, upload_id);
        
        // 清理
        cleanup_test_file(&test_file).await;
        manager.shutdown().await.unwrap();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_pause_resume_task() {
        let config = TransferConfig::default();
        let (manager, handle) = TransferManager::new(config);
        
        // 创建测试文件
        let test_file = create_test_file("test_pause.bin", 1024).await.unwrap();
        
        // 添加任务
        let upload_id = manager.add_upload(
            test_file.clone(),
            UploadStrategy::Simple(SimpleConfig::default()),
            Default::default(),
        ).await.unwrap();
        
        // 等待任务开始
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        // 暂停任务
        manager.pause(upload_id).await.unwrap();
        
        let task = manager.get_task(upload_id).await.unwrap();
        assert_eq!(task.state, UploadState::Paused);
        
        // 恢复任务
        manager.resume(upload_id).await.unwrap();
        
        let task = manager.get_task(upload_id).await.unwrap();
        assert_eq!(task.state, UploadState::Queued);
        
        // 清理
        cleanup_test_file(&test_file).await;
        manager.shutdown().await.unwrap();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_cancel_task() {
        let config = TransferConfig::default();
        let (manager, handle) = TransferManager::new(config);
        
        // 创建测试文件
        let test_file = create_test_file("test_cancel.bin", 1024).await.unwrap();
        
        // 添加任务
        let upload_id = manager.add_upload(
            test_file.clone(),
            UploadStrategy::Simple(SimpleConfig::default()),
            Default::default(),
        ).await.unwrap();
        
        // 取消任务
        manager.cancel(upload_id).await.unwrap();
        
        let task = manager.get_task(upload_id).await.unwrap();
        assert_eq!(task.state, UploadState::Cancelled);
        
        // 清理
        cleanup_test_file(&test_file).await;
        manager.shutdown().await.unwrap();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_event_subscription() {
        let config = TransferConfig::default();
        let (manager, handle) = TransferManager::new(config);
        
        // 订阅事件
        let mut receiver = manager.subscribe_events();
        
        // 创建测试文件
        let test_file = create_test_file("test_events.bin", 1024).await.unwrap();
        
        // 添加任务
        let upload_id = manager.add_upload(
            test_file.clone(),
            UploadStrategy::Simple(SimpleConfig::default()),
            Default::default(),
        ).await.unwrap();
        
        // 接收事件
        let event = receiver.recv().await.unwrap();
        match event {
            UploadEvent::TaskAdded { upload_id: id } => {
                assert_eq!(id, upload_id);
            }
            _ => panic!("Expected TaskAdded event"),
        }
        
        // 清理
        cleanup_test_file(&test_file).await;
        manager.shutdown().await.unwrap();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_retry_logic() {
        use crate::utils::{retry, RetryBuilder, RetryStrategy};
        
        let mut attempt = 0;
        let result = retry(|| async {
            attempt += 1;
            if attempt < 3 {
                Err(TransferError::Timeout)
            } else {
                Ok(42)
            }
        }).await;
        
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempt, 3);
    }

    #[tokio::test]
    async fn test_progress_tracker() {
        use crate::utils::ProgressTracker;
        
        let tracker = ProgressTracker::new(1000);
        
        // 更新进度
        tracker.add_bytes(100).await;
        tracker.add_bytes(200).await;
        
        let progress = tracker.get_progress().await;
        assert_eq!(progress.uploaded_bytes, 300);
        assert_eq!(progress.total_bytes, 1000);
        assert_eq!(progress.percentage, 30.0);
    }

    #[tokio::test]
    async fn test_format_utils() {
        use crate::utils::{format_bytes, format_speed, format_duration};
        
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1048576), "1.00 MB");
        assert_eq!(format_bytes(1073741824), "1.00 GB");
        
        assert_eq!(format_speed(1024.0), "1.00 KB/s");
        assert_eq!(format_speed(1048576.0), "1.00 MB/s");
        
        assert_eq!(format_duration(Duration::from_secs(59)), "59s");
        assert_eq!(format_duration(Duration::from_secs(120)), "2m 0s");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1h 1m 1s");
    }
}
