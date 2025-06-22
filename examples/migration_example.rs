use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// 这个示例展示如何从旧的 Tus API 迁移到新的统一 API
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Migration Example: From Old Tus API to New Transfer API\n");
    
    // ========== 旧 API 使用方式 ==========
    println!("=== Old Tus API ===");
    use tunnel::tus::{TusClient, UploadManager};
    
    let tus_client = TusClient::new("https://example.com/tus", 5 * 1024 * 1024);
    let tus_config = tunnel::tus::types::UploadConfig::default();
    let tus_manager_handle = UploadManager::new(tus_client, tus_config);
    
    // 使用旧 API 添加上传
    let old_upload_id = tus_manager_handle.manager
        .add_upload(PathBuf::from("old_api_file.txt"), None)
        .await?;
    
    println!("Old API upload started: {}", old_upload_id);
    
    // 关闭旧管理器
    tus_manager_handle.shutdown().await?;
    
    println!("\n=== New Transfer API ===");
    
    // ========== 新 API 使用方式 ==========
    use tunnel::{
        TransferManagerBuilder, TransferConfig,
        UploadStrategy, TusConfig,
        TusUploader,
    };
    
    // 创建新的传输管理器
    let (new_manager, handle) = TransferManagerBuilder::new()
        .config(TransferConfig::default())
        .register_uploader(
            "tus",
            Arc::new(TusUploader::new("https://example.com/tus", 5 * 1024 * 1024))
        )
        .build()
        .await?;
    
    // 使用新 API 添加上传
    let new_upload_id = new_manager.add_upload(
        PathBuf::from("new_api_file.txt"),
        UploadStrategy::Tus(TusConfig::default()),
        HashMap::new(),
    ).await?;
    
    println!("New API upload started: {}", new_upload_id);
    
    // ========== 功能对比 ==========
    println!("\n=== Feature Comparison ===");
    
    println!("Old API:");
    println!("  - Only supports Tus protocol");
    println!("  - Fixed configuration");
    println!("  - Limited extensibility");
    
    println!("\nNew API:");
    println!("  - Supports multiple protocols (Tus, Simple, Chunked)");
    println!("  - Flexible configuration");
    println!("  - Plugin architecture");
    println!("  - Progress callbacks");
    println!("  - Storage adapters");
    println!("  - Better error handling");
    
    // ========== 迁移指南 ==========
    println!("\n=== Migration Guide ===");
    println!("1. Replace TusClient with TusUploader");
    println!("2. Replace UploadManager with TransferManager");
    println!("3. Use UploadStrategy to specify protocol");
    println!("4. Add metadata as HashMap instead of Option<HashMap>");
    println!("5. Register uploaders before use");
    
    // 关闭新管理器
    new_manager.shutdown().await?;
    handle.await?;
    
    println!("\nMigration example completed!");
    Ok(())
}
