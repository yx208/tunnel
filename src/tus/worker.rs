use std::sync::Arc;
use tokio::fs::File;
use tokio_util::io::ReaderStream;
use tokio_util::sync::CancellationToken;
use crate::tus::progress_aggregator::{ProgressAggregator};
use crate::tus::progress_stream::AggregatedProgressStream;
use super::task::UploadTask;
use super::errors::{Result, TusError};
use super::client::TusClient;
use super::types::{UploadConfig};

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

        // 续传
        let offset = self.client.get_upload_offset(upload_url).await?;
        if offset >= file_size {
            return Ok(upload_url.to_string());
        }
        
        // 创建进度回调
        if let Some(aggregator) = &self.progress_aggregator {
            aggregator.register_task(task.id, file_size);
            // 更新初始进度
            if offset >= 0 {
                aggregator.update_task_progress(task.id, offset);
            }
        }

        let file = File::open(task.file_path).await?;
        let file_stream = ReaderStream::with_capacity(file, self.config.chunk_size);
        let body = if let Some(aggregator) = &self.progress_aggregator {
            let progress_stream = AggregatedProgressStream::new(
                file_stream,
                task.id,
                aggregator.clone(),
                offset,
            );
            reqwest::Body::wrap_stream(progress_stream)
        } else {
            reqwest::Body::wrap_stream(file_stream)
        };
        
        let future = self.client
            .upload_file_streaming(upload_url, file_size, offset, body);

        // 执行
        tokio::select! {
            result = future => result,
            _ = self.cancellation_token.cancelled() => {
                Err(TusError::Cancelled)
            }
        }
    }
}

