use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tokio::sync::{mpsc};
use super::errors::{Result, TusError};
use super::progress::{ProgressCallback, ProgressInfo};
use super::client::TusClient;
use super::manager::{UploadId, UploadProgress, UploadState, UploadTask};

pub struct UploadWorker {
    pub(crate) client: TusClient,
    pub(crate) cancellation_token: CancellationToken
}

impl UploadWorker {
    pub async fn run(
        self,
        task: UploadTask,
        progress_tx: mpsc::UnboundedSender<UploadProgress>,
        state_tx: mpsc::Sender<(UploadId, UploadState)>
    ) -> Result<String> {
        let file_size = task.file_size;
        let upload_url = task.upload_url.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Upload URL not set")
        })?;

        // 创建进度回调
        let upload_id = task.id;
        let progress_callback = Arc::new(move |info: ProgressInfo| {
            let _ = progress_tx.send(UploadProgress {
                upload_id,
                bytes_uploaded: info.bytes_uploaded,
                total_bytes: info.total_bytes,
                instant_speed: info.instant_speed,
                average_speed: info.average_speed,
                percentage: info.percentage,
                eta: info.eta,
            });
        });

        // 执行
        tokio::select! {
            result = self.upload_with_progress(upload_url, &task.file_path, file_size, progress_callback) => {
                result
            },
            _ = self.cancellation_token.cancelled() => {
                Err(TusError::Cancelled)
            }
        }
    }

    async fn upload_with_progress(
        &self,
        upload_url: &str,
        file_path: &PathBuf,
        file_size: u64,
        progress_callback: ProgressCallback
    ) -> Result<String> {
        self.client.upload_file_streaming(
            upload_url,
            file_path.to_str().unwrap(),
            file_size,
            Some(progress_callback),
        ).await?;

        Ok(upload_url.to_string())
    }

}

