use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tunnel::{
    TransferManager, TransferManagerBuilder, TransferConfig, TransferError,
    UploadStrategy, TusConfig, SimpleConfig, ChunkedConfig,
    TusUploader, SimpleUploader, ChunkedUploader,
    UploadEvent, UploadState,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Transfer Library Example");
    
    // 创建配置
    let config = TransferConfig {
        max_concurrent: 3,
        queue_size: 100,
        progress_interval: Duration::from_millis(500),
        enable_persistence: true,
        state_file: Some(PathBuf::from("transfer_state.json")),
        default_strategy: UploadStrategy::Tus(TusConfig::default()),
    };
    
    // 构建传输管理器
    let (mut manager, handle) = TransferManagerBuilder::new()
        .config(config)
        // 注册 Tus 上传器
        .register_uploader(
            "tus",
            Arc::new(TusUploader::new("https://example.com/tus", 5 * 1024 * 1024))
        )
        // 注册简单上传器
        .register_uploader(
            "simple",
            Arc::new(SimpleUploader::new("https://example.com/upload", SimpleConfig::default()))
        )
        // 注册分片上传器
        .register_uploader(
            "chunked",
            Arc::new(ChunkedUploader::new("https://example.com/multipart", ChunkedConfig::default()))
        )
        .build()
        .await?;
    
    // 订阅事件
    let mut event_receiver = manager.subscribe_events();
    
    // 启动事件处理任务
    tokio::spawn(async move {
        while let Ok(event) = event_receiver.recv().await {
            match event {
                UploadEvent::TaskAdded { upload_id } => {
                    println!("Task added: {}", upload_id);
                }
                UploadEvent::StateChanged { upload_id, old_state, new_state } => {
                    println!("Task {} state changed: {:?} -> {:?}", upload_id, old_state, new_state);
                }
                UploadEvent::Progress { upload_id, progress } => {
                    println!(
                        "Task {} progress: {:.2}% ({}/{} bytes) @ {}",
                        upload_id,
                        progress.percentage,
                        progress.uploaded_bytes,
                        progress.total_bytes,
                        tunnel::utils::format_speed(progress.speed)
                    );
                }
                UploadEvent::Completed { upload_id, url } => {
                    println!("Task {} completed: {}", upload_id, url);
                }
                UploadEvent::Failed { upload_id, error } => {
                    println!("Task {} failed: {}", upload_id, error);
                }
            }
        }
    });
    
    // 示例1：使用 Tus 协议上传
    let file_path = PathBuf::from("example.txt");
    let mut metadata = HashMap::new();
    metadata.insert("filename".to_string(), "example.txt".to_string());
    metadata.insert("content-type".to_string(), "text/plain".to_string());
    
    let upload_id = manager.add_upload(
        file_path.clone(),
        UploadStrategy::Tus(TusConfig::default()),
        metadata.clone(),
    ).await?;
    
    println!("Started Tus upload: {}", upload_id);
    
    // 示例2：使用简单上传
    let upload_id2 = manager.add_upload(
        file_path.clone(),
        UploadStrategy::Simple(SimpleConfig::default()),
        metadata.clone(),
    ).await?;
    
    println!("Started simple upload: {}", upload_id2);
    
    // 示例3：使用分片上传
    let upload_id3 = manager.add_upload(
        file_path.clone(),
        UploadStrategy::Chunked(ChunkedConfig {
            chunk_size: 1024 * 1024, // 1MB chunks
            concurrent_chunks: 5,
            max_retries: 3,
        }),
        metadata.clone(),
    ).await?;
    
    println!("Started chunked upload: {}", upload_id3);
    
    // 等待一段时间
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // 暂停第一个任务
    manager.pause(upload_id).await?;
    println!("Paused task: {}", upload_id);
    
    // 获取所有任务状态
    let tasks = manager.get_all_tasks().await;
    println!("\nCurrent tasks:");
    for task in tasks {
        println!("  {} - {:?} - {}%", 
            task.id, 
            task.state,
            (task.uploaded_bytes as f64 / task.file_size as f64 * 100.0) as u32
        );
    }
    
    // 恢复任务
    tokio::time::sleep(Duration::from_secs(1)).await;
    manager.resume(upload_id).await?;
    println!("Resumed task: {}", upload_id);
    
    // 批量添加文件
    let files = vec![
        (PathBuf::from("file1.txt"), Some(metadata.clone())),
        (PathBuf::from("file2.txt"), Some(metadata.clone())),
        (PathBuf::from("file3.txt"), None),
    ];
    
    let batch_ids = manager.add_uploads(files).await?;
    println!("\nBatch upload started: {:?}", batch_ids);
    
    // 等待所有任务完成或按 Ctrl+C 退出
    println!("\nPress Ctrl+C to exit...");
    tokio::signal::ctrl_c().await?;
    
    // 关闭管理器
    manager.shutdown().await?;
    handle.await?;
    
    Ok(())
}
