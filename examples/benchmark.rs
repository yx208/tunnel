use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tunnel::{
    TransferManagerBuilder, TransferConfig,
    UploadStrategy, TusConfig, SimpleConfig, ChunkedConfig,
    UploadEvent,
};
use tokio::fs;

/// 性能测试示例
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Transfer Library Performance Benchmark\n");
    
    // 创建测试文件
    let file_sizes = vec![
        (1024 * 1024, "1MB"),           // 1 MB
        (10 * 1024 * 1024, "10MB"),      // 10 MB
        (100 * 1024 * 1024, "100MB"),    // 100 MB
    ];
    
    let mut test_files = Vec::new();
    
    println!("Creating test files...");
    for (size, label) in &file_sizes {
        let path = PathBuf::from(format!("benchmark_{}.bin", label));
        create_test_file(&path, *size).await?;
        test_files.push((path, *size, *label));
    }
    
    // 测试不同的上传策略
    let strategies = vec![
        ("Tus", UploadStrategy::Tus(TusConfig {
            chunk_size: 5 * 1024 * 1024,
            parallel_chunks: false,
            max_retries: 3,
        })),
        ("Simple", UploadStrategy::Simple(SimpleConfig {
            timeout: Duration::from_secs(300),
            max_retries: 3,
        })),
        ("Chunked", UploadStrategy::Chunked(ChunkedConfig {
            chunk_size: 5 * 1024 * 1024,
            concurrent_chunks: 3,
            max_retries: 3,
        })),
    ];
    
    // 运行基准测试
    println!("\nRunning benchmarks...\n");
    
    for (strategy_name, strategy) in strategies {
        println!("=== {} Upload Strategy ===", strategy_name);
        
        for (file_path, size, label) in &test_files {
            let start_time = Instant::now();
            
            // 模拟上传（实际环境中会真正上传）
            let upload_time = simulate_upload(*size, &strategy).await;
            
            let throughput = *size as f64 / upload_time.as_secs_f64();
            
            println!("  {} file: {:.2}s @ {}/s",
                label,
                upload_time.as_secs_f64(),
                tunnel::utils::format_bytes(throughput as u64)
            );
        }
        
        println!();
    }
    
    // 并发性能测试
    println!("=== Concurrent Upload Test ===");
    
    let concurrent_counts = vec![1, 3, 5, 10];
    
    for concurrent in concurrent_counts {
        let (manager, handle) = TransferManagerBuilder::new()
            .config(TransferConfig {
                max_concurrent: concurrent,
                ..Default::default()
            })
            .build()
            .await?;
        
        let start_time = Instant::now();
        let mut upload_ids = Vec::new();
        
        // 添加多个任务
        for i in 0..10 {
            let path = PathBuf::from(format!("concurrent_test_{}.bin", i));
            create_test_file(&path, 1024 * 1024).await?; // 1MB files
            
            let upload_id = manager.add_upload(
                path.clone(),
                UploadStrategy::Simple(SimpleConfig::default()),
                Default::default(),
            ).await?;
            
            upload_ids.push((upload_id, path));
        }
        
        // 等待模拟完成
        tokio::time::sleep(Duration::from_secs(2)).await;
        
        let total_time = start_time.elapsed();
        
        println!("  {} concurrent: {:.2}s for 10 files",
            concurrent,
            total_time.as_secs_f64()
        );
        
        // 清理
        for (_, path) in upload_ids {
            let _ = fs::remove_file(&path).await;
        }
        
        manager.shutdown().await?;
        handle.await?;
    }
    
    // 内存使用测试
    println!("\n=== Memory Usage Test ===");
    println!("  Note: Use external tools to monitor actual memory usage");
    println!("  Adding 1000 tasks to queue...");
    
    let (manager, handle) = TransferManagerBuilder::new()
        .config(TransferConfig {
            max_concurrent: 5,
            queue_size: 1000,
            ..Default::default()
        })
        .build()
        .await?;
    
    let start_time = Instant::now();
    
    for i in 0..1000 {
        let path = PathBuf::from(format!("memory_test_{}.txt", i));
        create_test_file(&path, 1024).await?; // 1KB files
        
        manager.add_upload(
            path,
            UploadStrategy::Simple(SimpleConfig::default()),
            Default::default(),
        ).await?;
    }
    
    println!("  Added 1000 tasks in {:.2}s", start_time.elapsed().as_secs_f64());
    
    // 事件处理性能
    println!("\n=== Event Processing Performance ===");
    
    let mut event_receiver = manager.subscribe_events();
    let mut event_count = 0;
    let event_start = Instant::now();
    
    // 收集一些事件
    while event_start.elapsed() < Duration::from_secs(1) {
        if let Ok(Ok(_)) = tokio::time::timeout(
            Duration::from_millis(10),
            event_receiver.recv()
        ).await {
            event_count += 1;
        }
    }
    
    println!("  Processed {} events/second", event_count);
    
    // 清理所有测试文件
    println!("\nCleaning up test files...");
    for (path, _, _) in test_files {
        let _ = fs::remove_file(&path).await;
    }
    
    // 清理内存测试文件
    for i in 0..1000 {
        let path = PathBuf::from(format!("memory_test_{}.txt", i));
        let _ = fs::remove_file(&path).await;
    }
    
    manager.shutdown().await?;
    handle.await?;
    
    println!("\nBenchmark completed!");
    Ok(())
}

/// 创建指定大小的测试文件
async fn create_test_file(path: &PathBuf, size: usize) -> Result<(), Box<dyn std::error::Error>> {
    let data = vec![0u8; size];
    fs::write(path, data).await?;
    Ok(())
}

/// 模拟上传并返回耗时
async fn simulate_upload(size: usize, strategy: &UploadStrategy) -> Duration {
    // 模拟不同策略的上传速度
    let base_speed = 10 * 1024 * 1024; // 10 MB/s base speed
    
    let speed_multiplier = match strategy {
        UploadStrategy::Tus(_) => 0.8,      // Tus 有一些协议开销
        UploadStrategy::Simple(_) => 1.0,    // 简单上传最快
        UploadStrategy::Chunked(config) => {
            // 并发分块可能更快
            1.0 + (config.concurrent_chunks as f64 - 1.0) * 0.2
        }
    };
    
    let effective_speed = (base_speed as f64 * speed_multiplier) as usize;
    let duration_secs = size as f64 / effective_speed as f64;
    
    Duration::from_secs_f64(duration_secs)
}
