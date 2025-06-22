# Tunnel - 灵活的 Rust 传输库

一个功能强大且灵活的 Rust 文件传输库，支持多种上传协议，包括 Tus、简单 HTTP 上传和分片上传。

## 特性

- **多协议支持**：
  - **Tus 协议**：支持断点续传的可靠上传协议
  - **简单上传**：标准 HTTP POST/PUT 上传
  - **分片上传**：并发分片上传，适合大文件

- **统一接口**：所有上传方式使用相同的 API，易于切换和扩展

- **高级功能**：
  - 并发控制和任务队列
  - 实时进度跟踪
  - 断点续传支持
  - 自动重试机制
  - 状态持久化
  - 事件订阅系统

- **可扩展架构**：
  - 插件式上传器设计
  - 自定义进度回调
  - 灵活的存储适配器
  - 请求拦截器支持

## 安装

在 `Cargo.toml` 中添加：

```toml
[dependencies]
tunnel = "0.1.0"
```

## 快速开始

### 基本使用

```rust
use tunnel::{
    TransferManager, TransferConfig,
    UploadStrategy, TusConfig,
    TusUploader,
};
use std::sync::Arc;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 创建传输管理器
    let config = TransferConfig::default();
    let (manager, handle) = TransferManager::new(config);
    
    // 注册 Tus 上传器
    manager.register_uploader(
        "tus",
        Arc::new(TusUploader::new("https://example.com/tus", 5 * 1024 * 1024))
    ).await?;
    
    // 上传文件
    let upload_id = manager.add_upload(
        PathBuf::from("file.pdf"),
        UploadStrategy::Tus(TusConfig::default()),
        Default::default(),
    ).await?;
    
    println!("Upload started: {}", upload_id);
    
    // 等待完成...
    Ok(())
}
```

### 使用不同的上传策略

```rust
// Tus 协议上传（支持断点续传）
let upload_id = manager.add_upload(
    file_path.clone(),
    UploadStrategy::Tus(TusConfig {
        chunk_size: 5 * 1024 * 1024,  // 5MB 分块
        parallel_chunks: false,
        max_retries: 3,
    }),
    metadata,
).await?;

// 简单 HTTP 上传
let upload_id = manager.add_upload(
    file_path.clone(),
    UploadStrategy::Simple(SimpleConfig {
        timeout: Duration::from_secs(300),
        max_retries: 3,
    }),
    metadata,
).await?;

// 分片上传
let upload_id = manager.add_upload(
    file_path.clone(),
    UploadStrategy::Chunked(ChunkedConfig {
        chunk_size: 10 * 1024 * 1024,  // 10MB 分块
        concurrent_chunks: 5,           // 5 个并发分块
        max_retries: 3,
    }),
    metadata,
).await?;
```

### 任务控制

```rust
// 暂停上传
manager.pause(upload_id).await?;

// 恢复上传
manager.resume(upload_id).await?;

// 取消上传
manager.cancel(upload_id).await?;

// 获取任务信息
if let Some(task) = manager.get_task(upload_id).await {
    println!("Task state: {:?}", task.state);
    println!("Progress: {}%", 
        (task.uploaded_bytes as f64 / task.file_size as f64 * 100.0)
    );
}
```

### 事件订阅

```rust
// 订阅所有事件
let mut events = manager.subscribe_events();

tokio::spawn(async move {
    while let Ok(event) = events.recv().await {
        match event {
            UploadEvent::Progress { upload_id, progress } => {
                println!("Progress: {} - {:.2}%", upload_id, progress.percentage);
            }
            UploadEvent::Completed { upload_id, url } => {
                println!("Completed: {} -> {}", upload_id, url);
            }
            UploadEvent::Failed { upload_id, error } => {
                eprintln!("Failed: {} - {}", upload_id, error);
            }
            _ => {}
        }
    }
});
```

## 高级功能

### 使用 TransferManagerBuilder

```rust
use tunnel::{TransferManagerBuilder, TransferConfig};

let (manager, handle) = TransferManagerBuilder::new()
    .config(TransferConfig {
        max_concurrent: 5,
        queue_size: 100,
        progress_interval: Duration::from_millis(100),
        ..Default::default()
    })
    .register_uploader("tus", tus_uploader)
    .register_uploader("simple", simple_uploader)
    .register_uploader("chunked", chunked_uploader)
    .progress_callback(Arc::new(MyProgressCallback))
    .storage_adapter(Arc::new(FileStorageAdapter::new("state.json")))
    .build()
    .await?;
```

### 自定义进度回调

```rust
use async_trait::async_trait;
use tunnel::core::ProgressCallback;

struct MyProgressCallback;

#[async_trait]
impl ProgressCallback for MyProgressCallback {
    async fn on_progress(&self, upload_id: UploadId, progress: &UploadProgress) {
        // 自定义进度处理
        update_ui(upload_id, progress.percentage);
    }
    
    async fn on_completed(&self, upload_id: UploadId, url: &str) {
        // 上传完成处理
        notify_completion(upload_id, url);
    }
    
    // ... 其他回调方法
}
```

### 状态持久化

```rust
use tunnel::core::StorageAdapter;

// 使用文件系统存储
let storage = Arc::new(FileStorageAdapter::new("upload_state.json"));
manager.set_storage_adapter(storage);

// 或实现自定义存储适配器
struct DatabaseStorageAdapter {
    db: Database,
}

#[async_trait]
impl StorageAdapter for DatabaseStorageAdapter {
    async fn save_task(&self, task: &UploadTask) -> Result<()> {
        self.db.save_upload_task(task).await
    }
    
    // ... 其他方法
}
```

### 创建自定义上传器

```rust
use async_trait::async_trait;
use tunnel::core::{Uploader, UploadTask, UploadProgress, Result};

struct MyCustomUploader {
    endpoint: String,
}

#[async_trait]
impl Uploader for MyCustomUploader {
    async fn upload(&self, task: &UploadTask) -> Result<String> {
        // 实现上传逻辑
        todo!()
    }
    
    async fn resume(&self, task: &UploadTask) -> Result<String> {
        // 实现断点续传
        todo!()
    }
    
    // ... 其他方法
}

// 注册自定义上传器
manager.register_uploader(
    "custom",
    Arc::new(MyCustomUploader { endpoint: "https://api.example.com".into() })
).await?;
```

## 架构设计

```
tunnel/
├── core/              # 核心抽象和接口
│   ├── traits.rs      # Uploader, ProgressCallback 等 trait
│   ├── types.rs       # 共享类型定义
│   ├── manager.rs     # TransferManager 实现
│   └── errors.rs      # 错误类型
├── uploaders/         # 具体上传器实现
│   ├── tus/           # Tus 协议实现
│   ├── simple/        # 简单 HTTP 上传
│   └── chunked/       # 分片上传
└── utils/             # 工具模块
    ├── progress.rs    # 进度跟踪工具
    └── retry.rs       # 重试机制
```

## 示例

查看 `examples/` 目录获取更多示例：

- `transfer_example.rs` - 基本使用示例
- `advanced_example.rs` - 高级功能示例
- `storage_adapters.rs` - 存储适配器实现

## 从旧版本迁移

如果您正在使用旧的 Tus-only API，现有代码仍然可以工作：

```rust
// 旧代码仍然兼容
use tunnel::tus::{TusClient, UploadManager};

// 推荐迁移到新 API
use tunnel::{TransferManager, TusUploader};
```

## 性能建议

1. **选择合适的分块大小**：
   - 小文件：1-5 MB
   - 大文件：5-10 MB
   - 超大文件：10-50 MB

2. **并发控制**：
   - 根据网络带宽调整 `max_concurrent`
   - 避免过多并发导致内存占用过高

3. **重试策略**：
   - 使用指数退避避免服务器过载
   - 根据错误类型决定是否重试

## 贡献

欢迎提交 Issue 和 Pull Request！

## 许可证

MIT License
