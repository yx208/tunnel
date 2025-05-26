use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use futures_util::TryFutureExt;
use tokio::sync::{oneshot, mpsc, RwLock};
use tokio::task::JoinHandle;
use super::task::UploadTask;
use super::manager_worker::UploadManagerWorker;
use super::types::{ManagerCommand, UploadEvent, UploadId};
use super::errors::{Result, TusError};
use super::client::TusClient;

pub struct UploadManager {
    command_tx: mpsc::Sender<ManagerCommand>,
    event_rx: Arc<RwLock<mpsc::UnboundedReceiver<UploadEvent>>>,
    worker_handle: JoinHandle<()>,
}

impl UploadManager {
    pub fn new(client: TusClient, concurrent: usize, state_file: Option<PathBuf>) -> Self {
        let (command_tx, command_rx) = mpsc::channel(100);
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let worker_handle = tokio::spawn(UploadManagerWorker::run(
            client,
            concurrent,
            state_file,
            command_rx,
            event_tx
        ));

        Self {
            command_tx,
            event_rx: Arc::new(RwLock::new(event_rx)),
            worker_handle,
        }
    }

    /// Add upload task
    pub async fn add_upload(&self, file_path: PathBuf, metadata: Option<HashMap<String, String>>)
        -> Result<UploadId>
    {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(ManagerCommand::AddUpload {
                file_path: file_path.into(),
                metadata,
                reply: reply_tx
            })
            .await
            .map_err(|_| TusError::internal_error("Manager shut down"))?;

        // 等待响应
        reply_rx
            .await
            .map_err(|err| TusError::internal_error(err.to_string()))?
    }

    /// Pause upload task
    pub async fn pause_upload(&self, upload_id: UploadId) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        
        self.command_tx
            .send(ManagerCommand::PauseUpload {
                upload_id,
                reply: reply_tx
            })
            .await
            .map_err(|_| TusError::internal_error("Manager shut down"))?;

        reply_rx
            .await
            .map_err(|err| TusError::internal_error(err.to_string()))?
    }

    /// Resume upload
    pub async fn resume_upload(&self, upload_id: UploadId) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(ManagerCommand::ResumeUpload {
                upload_id,
                reply: reply_tx
            })
            .await
            .map_err(|_| TusError::internal_error("Manager shut down"))?;

        reply_rx
            .await
            .map_err(|err| TusError::internal_error(err.to_string()))?
    }

    /// Cancel upload
    pub async fn cancel_upload(&self, upload_id: UploadId) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(ManagerCommand::CancelUpload {
                upload_id,
                reply: reply_tx
            })
            .await
            .map_err(|_| TusError::internal_error("Manager shut down"))?;

        reply_rx
            .await
            .map_err(|err| TusError::internal_error(err.to_string()))?
    }

    /// Get task
    pub async fn get_task(&self, upload_id: UploadId) -> Result<Option<UploadTask>> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(ManagerCommand::GetTask {
                upload_id,
                reply: reply_tx
            })
            .await
            .map_err(|_| TusError::internal_error("Manager shut down"))?;

        let task = reply_rx
            .await
            .map_err(|err| TusError::internal_error(err.to_string()))?;

        Ok(task)
    }

    /// Get all task
    pub async fn get_all_tasks(&self) -> Result<Vec<UploadTask>> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(ManagerCommand::GetAllTasks { reply: reply_tx })
            .await
            .map_err(|_| TusError::internal_error("Manager shut down"))?;

        let tasks = reply_rx
            .await
            .map_err(|err| TusError::internal_error(err.to_string()))?;

        Ok(tasks)
    }
    
    pub async fn subscribe_events(&self) -> mpsc::UnboundedReceiver<UploadEvent> {
        todo!("Implement proper event broadcasting")
    }
}
