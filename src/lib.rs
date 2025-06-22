#![allow(warnings)]

pub mod core;
pub mod uploaders;
pub mod utils;
pub mod config;

// 保留原有的 tus 模块以保持向后兼容
pub mod tus;

// 重新导出核心类型
pub use core::{
    TransferManager,
    TransferManagerBuilder,
    UploadId,
    UploadState,
    UploadStrategy,
    UploadTask,
    UploadProgress,
    TransferConfig,
    TransferError,
    Result,
};

// 重新导出上传器
pub use uploaders::{
    TusUploader,
    SimpleUploader,
    ChunkedUploader,
};

#[cfg(test)]
mod tests;