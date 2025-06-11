use std::mem::take;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tokio::sync::mpsc;
use crate::tus::progress_aggregator::{create_task_progress_callback, ProgressAggregator};
use super::task::UploadTask;
use super::errors::{Result, TusError};
use super::progress::{ProgressInfo};
use super::client::TusClient;
use super::types::{TaskProgress, UploadConfig};

pub struct UploadWorker {
    pub(crate) client: TusClient,
    pub(crate) cancellation_token: CancellationToken,
    pub(crate) config: UploadConfig,
    pub(crate) progress_aggregator: Option<Arc<ProgressAggregator>>,
}

impl UploadWorker {
    pub async fn run(self, task: UploadTask) -> Result<String> {
        let file_size = task.file_size;
        let upload_url = task.upload_url.as_ref().ok_or_else(|| {
            TusError::ParamError("Upload URL not set".to_string())
        })?;
        
        // 如果使用批量进度，注册任务到聚合器
        if let Some(ref aggregator) = self.progress_aggregator {
            aggregator.register_task(task.id, file_size);
        }

        // 创建进度回调
        let progress_callback = if let Some(aggregator) = &self.progress_aggregator {
            Some(create_task_progress_callback(aggregator.clone(), task.id))
        } else {
            None
        };

        let future = self.client
            .upload_file_streaming(
                upload_url,
                &task.file_path.to_str().unwrap(),
                file_size,
                progress_callback
            );

        // 执行
        tokio::select! {
            result = future => result,
            _ = self.cancellation_token.cancelled() => {
                Err(TusError::Cancelled)
            }
        }
    }
}

