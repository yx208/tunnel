use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tunnel::{
    TransferManagerBuilder, TransferConfig, 
    UploadStrategy, TusConfig,
    TusUploader,
    UploadId, UploadProgress,
    core::{ProgressCallback, StorageAdapter},
};

// 包含存储适配器示例
mod storage_adapters;
use storage_adapters::FileStorageAdapter;

/// 自定义进度回调实现
struct ConsoleProgressCallback;

#[async_trait]
impl ProgressCallback for ConsoleProgressCallback {
    async fn on_progress(&self, upload_id: UploadId, progress: &UploadProgress) {
        println!(
            "[PROGRESS] {} - {:.2}% ({}/{} bytes) @ {} - ETA: {}",
            upload_id,
            progress.percentage,
            tunnel::utils::format_bytes(progress.uploaded_bytes),
            tunnel::utils::format_bytes(progress.total_bytes),
            tunnel::utils::format_speed(progress.speed),
            progress.eta
                .map(|d| tunnel::utils::format_duration(d))
                .unwrap_or_else(|| "N/A".to_string())
        );
    }
    
    async fn on_state_change(&self, upload_id: UploadId, old_state: &str, new_state: &str) {
        println!("[STATE] {} - {} -> {}", upload_id, old_state, new_state);
    }
    
    async fn on_completed(&self, upload_id: UploadId, url: &str) {
        println!("[COMPLETED] {} - URL: {}", upload_id, url);
    }
    
    async fn on_failed(&self, upload_id: UploadId, error: &str) {
        eprintln!("[FAILED] {} - Error: {}", upload_id, error);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Advanced Transfer Example with Progress and Storage\n");
    
    // 创建存储适配器
    let storage = Arc::new(FileStorageAdapter::new(PathBuf::from("transfer_state.json")));
    
    // 创建进度回调
    let progress_callback = Arc::new(ConsoleProgressCallback);
    
    // 配置
    let config = TransferConfig {
        max_concurrent: 2,
        queue_size: 50,
        progress_interval: Duration::from_millis(100),
        enable_persistence: true,
        state_file: Some(PathBuf::from("transfer_state.json")),
        default_strategy: UploadStrategy::Tus(TusConfig::default()),
    };
    
    // 构建传输管理器
    let (manager, handle) = TransferManagerBuilder::new()
        .config(config)
        .register_uploader(
            "tus",
            Arc::new(TusUploader::new("https://example.com/tus", 5 * 1024 * 1024))
        )
        .progress_callback(progress_callback)
        .storage_adapter(storage.clone())
        .build()
        .await?;
    
    // 检查是否有未完成的任务
    let existing_tasks = storage.list_tasks().await?;
    if !existing_tasks.is_empty() {
        println!("Found {} existing tasks:", existing_tasks.len());
        for task in &existing_tasks {
            println!("  {} - {:?} - {}%", 
                task.id,
                task.state,
                (task.uploaded_bytes as f64 / task.file_size as f64 * 100.0) as u32
            );
        }
        
        // 恢复未完成的任务
        for task in existing_tasks {
            if matches!(task.state, tunnel::UploadState::Paused | tunnel::UploadState::Queued) {
                println!("Resuming task: {}", task.id);
                manager.resume(task.id).await?;
            }
        }
    }
    
    // 添加新任务
    let files = vec![
        "large_file.bin",
        "document.pdf",
        "image.jpg",
    ];
    
    let mut metadata = HashMap::new();
    metadata.insert("user".to_string(), "example_user".to_string());
    metadata.insert("project".to_string(), "test_project".to_string());
    
    println!("\nAdding new upload tasks:");
    let mut upload_ids = Vec::new();
    
    for file in files {
        let path = PathBuf::from(file);
        if path.exists() {
            let upload_id = manager.add_upload(
                path,
                UploadStrategy::Tus(TusConfig {
                    chunk_size: 1024 * 1024, // 1MB chunks
                    parallel_chunks: false,
                    max_retries: 5,
                }),
                metadata.clone(),
            ).await?;
            
            println!("  Added: {} -> {}", file, upload_id);
            upload_ids.push(upload_id);
        } else {
            println!("  Skipped: {} (file not found)", file);
        }
    }
    
    // 模拟用户交互
    println!("\nCommands:");
    println!("  p <id> - Pause upload");
    println!("  r <id> - Resume upload");
    println!("  c <id> - Cancel upload");
    println!("  s      - Show all tasks");
    println!("  q      - Quit");
    
    // 启动命令处理循环
    let manager_clone = manager.clone();
    tokio::spawn(async move {
        let stdin = std::io::stdin();
        let mut line = String::new();
        
        loop {
            line.clear();
            if stdin.read_line(&mut line).is_err() {
                break;
            }
            
            let parts: Vec<&str> = line.trim().split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }
            
            match parts[0] {
                "p" => {
                    if parts.len() > 1 {
                        if let Ok(id) = parts[1].parse::<uuid::Uuid>() {
                            let upload_id = UploadId(id);
                            match manager_clone.pause(upload_id).await {
                                Ok(_) => println!("Paused: {}", upload_id),
                                Err(e) => eprintln!("Error pausing: {}", e),
                            }
                        }
                    }
                }
                "r" => {
                    if parts.len() > 1 {
                        if let Ok(id) = parts[1].parse::<uuid::Uuid>() {
                            let upload_id = UploadId(id);
                            match manager_clone.resume(upload_id).await {
                                Ok(_) => println!("Resumed: {}", upload_id),
                                Err(e) => eprintln!("Error resuming: {}", e),
                            }
                        }
                    }
                }
                "c" => {
                    if parts.len() > 1 {
                        if let Ok(id) = parts[1].parse::<uuid::Uuid>() {
                            let upload_id = UploadId(id);
                            match manager_clone.cancel(upload_id).await {
                                Ok(_) => println!("Cancelled: {}", upload_id),
                                Err(e) => eprintln!("Error cancelling: {}", e),
                            }
                        }
                    }
                }
                "s" => {
                    let tasks = manager_clone.get_all_tasks().await;
                    println!("\nCurrent tasks:");
                    for task in tasks {
                        println!("  {} - {:?} - {}% - {}", 
                            task.id,
                            task.state,
                            (task.uploaded_bytes as f64 / task.file_size as f64 * 100.0) as u32,
                            task.file_path.display()
                        );
                    }
                }
                "q" => {
                    println!("Shutting down...");
                    let _ = manager_clone.shutdown().await;
                    break;
                }
                _ => {
                    println!("Unknown command: {}", parts[0]);
                }
            }
        }
    });
    
    // 等待退出信号
    tokio::signal::ctrl_c().await?;
    
    // 保存最终状态并关闭
    println!("\nSaving state and shutting down...");
    manager.shutdown().await?;
    handle.await?;
    
    println!("Goodbye!");
    Ok(())
}
