use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use futures_util::TryFutureExt;
use tokio::sync::{oneshot, mpsc, broadcast};
use tokio::task::JoinHandle;
use super::progress_aggregator::{ProgressAggregator, ProgressAggregatorHandle};
use super::task::UploadTask;
use super::manager_worker::UploadManagerWorker;
use super::types::{AggregatedStats, ManagerCommand, UploadConfig, UploadEvent, UploadId};
use super::errors::{Result, TusError};
use super::client::TusClient;

/// 上传管理器
#[derive(Clone)]
pub struct UploadManager {
    /// 从外部接收处理任务的命令
    command_tx: mpsc::Sender<ManagerCommand>,

    /// 内部往外面抛出的事件
    event_tx: broadcast::Sender<UploadEvent>,

    /// 进度聚合器
    progress_aggregator: Arc<ProgressAggregator>,
}

/// 上传管理器句柄 - 包含管理器/工作线程/聚合器句柄
pub struct UploadManagerHandle {
    pub manager: UploadManager,
    worker_handle: JoinHandle<()>,
    aggregator_handle: Option<ProgressAggregatorHandle>,
}

impl UploadManagerHandle {
    pub async fn shutdown(self) -> Result<()> {
        drop(self.manager);

        // 关闭聚合器
        if let Some(aggregator_handle) = self.aggregator_handle {
            aggregator_handle.shutdown().await;
        }

        // 等待工作进程结束
        self.worker_handle.await
            .map_err(|err| TusError::Internal(format!("Worker panic: {}", err)))
    }
}

impl UploadManager {
    pub fn new(client: TusClient, config: UploadConfig) -> UploadManagerHandle {
        let (command_tx, command_rx) = mpsc::channel(100);
        let (event_tx, _) = broadcast::channel(512);

        // 批量更新通道
        let (batch_progress_tx, mut batch_progress_rx) = mpsc::unbounded_channel();

        // 创建聚合器
        let progress_aggregator = Arc::new(
            ProgressAggregator::new(batch_progress_tx)
                .with_update_interval(config.progress_interval)
        );
        let aggregator_handle = if config.batch_progress {
            let event_tx_clone = event_tx.clone();
            tokio::spawn(async move {
                while let Some(batch) = batch_progress_rx.recv().await {
                    let _ = event_tx_clone.send(UploadEvent::BatchProgress(batch));
                }
            });

            Some(progress_aggregator.clone().start())
        } else {
            None
        };

        // 工作线程
        let worker_handle = tokio::spawn(UploadManagerWorker::run(
            client,
            config.clone(),
            command_rx,
            event_tx.clone(),
            progress_aggregator.clone()
        ));

        let manager = Self {
            command_tx,
            event_tx,
            progress_aggregator
        };

        UploadManagerHandle {
            manager,
            worker_handle,
            aggregator_handle
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
            .map_err(|_| TusError::internal("Manager shut down"))?;

        // 等待响应
        reply_rx
            .await
            .map_err(|err| TusError::internal(err.to_string()))?
    }

    /// 批量添加
    pub async fn add_uploads(&self, files: Vec<(PathBuf, Option<HashMap<String, String>>)>)
        -> Result<Vec<UploadId>>
    {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(ManagerCommand::BatchAdd {
                files,
                reply: reply_tx
            })
            .await
            .map_err(|_| TusError::internal("Manager shut down"))?;

        reply_rx
            .await
            .map_err(|err| TusError::internal(err.to_string()))?
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
            .map_err(|_| TusError::internal("Manager shut down"))?;

        reply_rx
            .await
            .map_err(|err| TusError::internal(err.to_string()))?
    }

    pub async fn pause_all(&self) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(ManagerCommand::PauseAll { reply: reply_tx })
            .await
            .map_err(|_| TusError::internal("Manager shut down"))?;

        reply_rx
            .await
            .map_err(|err| TusError::internal(err.to_string()))?
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
            .map_err(|_| TusError::internal("Manager shut down"))?;

        reply_rx
            .await
            .map_err(|err| TusError::internal(err.to_string()))?
    }

    /// 恢复所有
    pub async fn resume_all(&self) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(ManagerCommand::ResumeAll { reply: reply_tx })
            .await
            .map_err(|_| TusError::internal("Manager shut down"))?;

        reply_rx
            .await
            .map_err(|err| TusError::internal(err.to_string()))?
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
            .map_err(|_| TusError::internal("Manager shut down"))?;

        reply_rx
            .await
            .map_err(|err| TusError::internal(err.to_string()))?
    }

    /// Cancel all
    pub async fn cancel_all(&self) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(ManagerCommand::CancelAll { reply: reply_tx })
            .await
            .map_err(|_| TusError::internal("Manager shut down"))?;

        reply_rx
            .await
            .map_err(|err| TusError::internal(err.to_string()))?
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
            .map_err(|_| TusError::internal("Manager shut down"))?;

        let task = reply_rx
            .await
            .map_err(|err| TusError::internal(err.to_string()))?;

        Ok(task)
    }

    /// Get all task
    pub async fn get_all_tasks(&self) -> Result<Vec<UploadTask>> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(ManagerCommand::GetAllTasks { reply: reply_tx })
            .await
            .map_err(|_| TusError::internal("Manager shut down"))?;

        let tasks = reply_rx
            .await
            .map_err(|err| TusError::internal(err.to_string()))?;

        Ok(tasks)
    }

    /// 获取当前统计信息（立即获取，不等待批量更新）
    pub async fn get_stats(&self) -> Result<Option<AggregatedStats>> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(ManagerCommand::GetStats { reply: reply_tx })
            .await
            .map_err(|_| TusError::internal("Manager shut down"))?;

        let result = reply_rx
            .await
            .map_err(|err| TusError::internal(err.to_string()))?;

        Ok(result)
    }

    /// 清理已完成/失败的任务
    pub async fn clean(&self) -> Result<usize> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(ManagerCommand::Clean { reply: reply_tx })
            .await
            .map_err(|_| TusError::internal("Manager shut down"))?;

        let count = reply_rx
            .await
            .map_err(|err| TusError::internal(err.to_string()))?;
        
        Ok(count)
    }

    pub async fn set_progress_interval(&self, interval: Duration) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(ManagerCommand::SetProgressInterval {
                interval,
                reply: reply_tx
            })
            .await
            .map_err(|_| TusError::internal("Manager shut down"))?;

        reply_rx
            .await
            .map_err(|err| TusError::internal(err.to_string()))?
    }

    /// 订阅所有事件
    pub fn subscribe_events(&self) -> broadcast::Receiver<UploadEvent> {
        self.event_tx.subscribe()
    }

    /// 订阅特定类型的事件
    pub fn subscribe_filtered<F>(&self, filter: F) -> FilteredEventReceiver<F>
    where
        F: Fn(&UploadEvent) -> bool
    {
        FilteredEventReceiver {
            receiver: self.event_tx.subscribe(),
            filter
        }
    }

    /// 仅订阅批量进度事件
    pub fn subscribe_progress(&self) -> FilteredEventReceiver<impl Fn(&UploadEvent) -> bool> {
        self.subscribe_filtered(|event| matches!(event, UploadEvent::BatchProgress(_)))
    }

    /// 仅订阅状态变更事件
    pub fn subscribe_state_changes(&self) -> FilteredEventReceiver<impl Fn(&UploadEvent) -> bool> {
        self.subscribe_filtered(|event| matches!(event, UploadEvent::StateChanged { .. }))
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

/// 便捷的构建器模式
pub struct UploadManagerBuilder {
    client: Option<TusClient>,
    config: UploadConfig,
}

impl UploadManagerBuilder {
    pub fn new() -> Self {
        Self {
            client: None,
            config: UploadConfig::default(),
        }
    }

    pub fn client(mut self, client: TusClient) -> Self {
        self.client = Some(client);
        self
    }

    pub fn config(mut self, config: UploadConfig) -> Self {
        self.config = config;
        self
    }

    pub fn max_concurrent(mut self, max: usize) -> Self {
        self.config.concurrent = max;
        self
    }

    pub fn progress_interval(mut self, interval: Duration) -> Self {
        self.config.progress_interval = interval;
        self
    }

    pub fn batch_progress(mut self, enabled: bool) -> Self {
        self.config.batch_progress = enabled;
        self
    }

    pub fn state_file(mut self, path: PathBuf) -> Self {
        self.config.state_file = Some(path);
        self
    }

    pub fn build(self) -> Result<UploadManagerHandle> {
        let client = self.client
            .ok_or_else(|| TusError::ParamError("Client not set".to_string()))?;

        Ok(UploadManager::new(client, self.config))
    }
}
