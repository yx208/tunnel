use std::collections::HashMap;
use std::path::PathBuf;
use futures_util::TryFutureExt;
use tokio::sync::{oneshot, mpsc, broadcast};
use tokio::task::JoinHandle;
use super::task::UploadTask;
use super::manager_worker::UploadManagerWorker;
use super::types::{ManagerCommand, UploadEvent, UploadId};
use super::errors::{Result, TusError};
use super::client::TusClient;

#[derive(Clone)]
pub struct UploadManager {
    command_tx: mpsc::Sender<ManagerCommand>,
    event_tx: broadcast::Sender<UploadEvent>,
}

/// 上传管理器句柄 - 包含管理器和工作线程
pub struct UploadManagerHandle {
    pub manager: UploadManager,
    pub worker_handle: JoinHandle<()>,
}

impl UploadManagerHandle {
    pub async fn shutdown(self) -> Result<()> {
        drop(self.manager);
        self.worker_handle.await
            .map_err(|err| TusError::InternalError(format!("Worker panic: {}", err)))
    }
}

impl UploadManager {
    pub fn new(client: TusClient, concurrent: usize, state_file: Option<PathBuf>) -> UploadManagerHandle {
        let (command_tx, command_rx) = mpsc::channel(100);
        // 最大缓存 256 个事件
        let (event_tx, _) = broadcast::channel(256);

        let worker_handle = tokio::spawn(UploadManagerWorker::run(
            client,
            concurrent,
            state_file,
            command_rx,
            event_tx.clone()
        ));

        let manager = Self {
            command_tx,
            event_tx,
        };

        UploadManagerHandle {
            manager,
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

    /// 订阅事件
    ///
    /// 注意：
    /// - 如果接收速度跟不上发送速度，可能会丢失事件（lagged error）
    /// - 每个订阅者都会收到完整的事件副本
    /// - 订阅者应该尽快处理事件，避免阻塞
    pub fn subscribe_events(&self) -> broadcast::Receiver<UploadEvent> {
        self.event_tx.subscribe()
    }

    pub fn subscribe_filtered<F>(&self, filter: F) -> FilteredEventReceiver<F> {
        FilteredEventReceiver {
            receiver: self.event_tx.subscribe(),
            filter
        }
    }
}

/// 过滤的事件接收器
pub struct FilteredEventReceiver<F> {
    receiver: broadcast::Receiver<UploadEvent>,
    filter: F,
}

impl<F> FilteredEventReceiver<F>
where
    F: Fn(&UploadEvent) -> bool,
{
    pub async fn recv(&mut self) -> Result<UploadEvent, broadcast::error::RecvError> {
        loop {
            let event = self.receiver.recv().await?;
            if (self.filter)(&event) {
                return Ok(event);
            }
        }
    }
}
