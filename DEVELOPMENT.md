# Tunnel 传输库开发文档

## 项目概述

Tunnel 是一个灵活、可扩展的 Rust 文件传输库，最初支持 Tus 协议，现已扩展为支持多种上传协议的统一传输框架。

## 架构设计

### 1. 核心抽象层 (`core/`)

- **traits.rs**: 定义核心接口
  - `Uploader`: 所有上传器必须实现的 trait
  - `ProgressCallback`: 进度回调接口
  - `StorageAdapter`: 状态存储接口
  - `RequestInterceptor`: 请求拦截器接口

- **types.rs**: 共享类型定义
  - `UploadId`: 任务唯一标识
  - `UploadState`: 任务状态枚举
  - `UploadStrategy`: 上传策略
  - `UploadTask`: 任务信息
  - `TransferConfig`: 管理器配置

- **manager.rs**: 传输管理器实现
  - 任务队列管理
  - 并发控制
  - 事件分发
  - 生命周期管理

- **errors.rs**: 错误类型定义

### 2. 上传器实现 (`uploaders/`)

#### Tus 上传器 (`tus/`)
- 支持断点续传
- 遵循 Tus 1.0.0 协议
- 分块上传

#### 简单上传器 (`simple/`)
- 标准 HTTP POST/PUT
- 适合小文件
- 不支持断点续传

#### 分片上传器 (`chunked/`)
- 并发分片上传
- 适合大文件
- 支持断点续传

### 3. 工具模块 (`utils/`)

- **progress.rs**: 进度跟踪和速度计算
- **retry.rs**: 重试机制实现

## 关键特性

### 1. 统一接口
所有上传方式使用相同的 API，便于切换和扩展。

```rust
// 只需改变策略即可切换上传方式
let strategy = UploadStrategy::Tus(config);     // Tus 协议
let strategy = UploadStrategy::Simple(config);   // 简单上传
let strategy = UploadStrategy::Chunked(config);  // 分片上传
```

### 2. 插件式架构
```rust
// 注册自定义上传器
manager.register_uploader("custom", Arc::new(MyUploader));
```

### 3. 事件系统
```rust
// 订阅特定事件
let mut progress_events = manager.subscribe_progress();
let mut state_events = manager.subscribe_state_changes();
```

### 4. 状态持久化
```rust
// 使用存储适配器保存任务状态
manager.set_storage_adapter(Arc::new(FileStorageAdapter::new("state.json")));
```

## 扩展指南

### 创建新的上传器

1. 实现 `Uploader` trait:

```rust
#[async_trait]
impl Uploader for MyUploader {
    async fn upload(&self, task: &UploadTask) -> Result<String> {
        // 实现上传逻辑
    }
    
    async fn resume(&self, task: &UploadTask) -> Result<String> {
        // 实现断点续传
    }
    
    // ... 其他方法
}
```

2. 注册到管理器:

```rust
manager.register_uploader("my_uploader", Arc::new(MyUploader::new()));
```

### 自定义进度回调

```rust
#[async_trait]
impl ProgressCallback for MyCallback {
    async fn on_progress(&self, upload_id: UploadId, progress: &UploadProgress) {
        // 处理进度更新
    }
}
```

### 实现存储适配器

```rust
#[async_trait]
impl StorageAdapter for MyStorage {
    async fn save_task(&self, task: &UploadTask) -> Result<()> {
        // 保存任务状态
    }
    
    async fn load_task(&self, upload_id: &UploadId) -> Result<Option<UploadTask>> {
        // 加载任务状态
    }
}
```

## 性能优化

### 1. 并发控制
- 根据网络带宽调整 `max_concurrent`
- 避免过多并发导致内存占用过高

### 2. 分块大小
- 小文件: 1-5 MB
- 大文件: 5-10 MB
- 超大文件: 10-50 MB

### 3. 重试策略
- 使用指数退避避免服务器过载
- 根据错误类型决定是否重试

## 测试

### 单元测试
```bash
cargo test
```

### 集成测试
```bash
cargo test --test integration_test
```

### 性能测试
```bash
cargo run --example benchmark
```

## 示例代码

- `examples/transfer_example.rs` - 基本使用示例
- `examples/advanced_example.rs` - 高级功能示例
- `examples/migration_example.rs` - 从旧 API 迁移
- `examples/benchmark.rs` - 性能测试

## API 兼容性

### 向后兼容
旧的 Tus API 仍然可用：
```rust
use tunnel::tus::{TusClient, UploadManager};
```

### 推荐迁移
建议迁移到新的统一 API：
```rust
use tunnel::{TransferManager, TusUploader};
```

## 未来计划

1. **更多协议支持**
   - S3 多部分上传
   - WebDAV
   - FTP/SFTP

2. **增强功能**
   - 带宽限制
   - 优先级队列
   - 断点续传优化

3. **监控和分析**
   - 详细的传输统计
   - 性能分析工具
   - 错误分析

## 贡献指南

1. Fork 项目
2. 创建功能分支
3. 提交更改
4. 创建 Pull Request

## 许可证

MIT License
