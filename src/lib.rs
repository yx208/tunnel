#![allow(warnings, warnings)]

pub mod config;
pub mod core;
pub mod protocol;

pub mod builder {
    use std::path::PathBuf;
    use crate::protocol::TusUploadBuilder;

    pub fn upload_tus(file: impl Into<PathBuf>, endpoint: impl Into<String>) -> TusUploadBuilder {
        TusUploadBuilder::new(file.into(), endpoint.into())
    }
}

pub mod presets {
    use std::time::Duration;
    use super::core::TransferConfig;

    pub fn default_config() -> TransferConfig {
        TransferConfig::default()
    }
    
    pub fn performance_config() -> TransferConfig {
        TransferConfig {
            max_concurrent: 10,
            buffer_size: 16 * 1024 * 1024,
            progress_interval: Duration::from_millis(500),
            batch_progress: true,
            ..Default::default()
        }
    }
    
    pub fn low_resource_config() -> TransferConfig {
        TransferConfig {
            max_concurrent: 3,
            buffer_size: 2 * 1024 * 1024,
            progress_interval: Duration::from_millis(1),
            batch_progress: false,
            ..Default::default()
        }
    }
}
