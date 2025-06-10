use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tokio::sync::mpsc;
use super::task::UploadTask;
use super::errors::{Result, TusError};
use super::progress::{ProgressInfo};
use super::client::TusClient;
use super::types::TaskProgress;

pub struct UploadWorker {
    pub(crate) client: TusClient,
    pub(crate) cancellation_token: CancellationToken
}

impl UploadWorker {
    pub async fn run(
        self,
        task: UploadTask,
        progress_tx: mpsc::UnboundedSender<TaskProgress>
    ) -> Result<String> {
        let file_size = task.file_size;
        let upload_url = task.upload_url.as_ref().ok_or_else(|| {
            TusError::ParamError("Upload URL not set".to_string())
        })?;

        // 创建进度回调
        let upload_id = task.id;
        let progress_callback = Arc::new(move |info: ProgressInfo| {
            let _ = progress_tx.send(TaskProgress {
                upload_id,
                bytes_uploaded: info.bytes_uploaded,
                total_bytes: info.total_bytes,
                instant_speed: info.instant_speed,
                average_speed: info.average_speed,
                percentage: info.percentage,
                eta: info.eta,
            });
        });

        let future = self.client
            .upload_file_streaming(
                upload_url,
                &task.file_path.to_str().unwrap(),
                file_size,
                Some(progress_callback)
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

