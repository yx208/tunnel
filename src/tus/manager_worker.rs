use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use super::types::TusClient;
use super::errors::Result;
use super::upload_manager::{ManagerCommand, UploadEvent, UploadId, UploadState, UploadTask};

struct TaskHandle {
    task: UploadTask,
    cancellation_token: Option<CancellationToken>,
    join_handle: Option<JoinHandle<Result<String>>>,
}

pub struct UploadManagerWorker {
    client: TusClient,
    max_concurrent: usize,
    tasks: HashMap<UploadId, TaskHandle>,
    queued_tasks: Vec<UploadId>,
    active_uploads: usize,
    event_tx: mpsc::UnboundedSender<UploadEvent>,
    state_file: Option<PathBuf>,
}

impl UploadManagerWorker {
    pub async fn run(
        client: TusClient,
        max_concurrent: usize,
        state_file: Option<PathBuf>,
        command_rx: mpsc::Receiver<ManagerCommand>,
        event_tx: mpsc::UnboundedSender<UploadEvent>
    ) {
        let mut worker = Self {
            client,
            max_concurrent,
            tasks: HashMap::new(),
            queued_tasks: Vec::new(),
            active_uploads: 0,
            event_tx,
            state_file,
        };
        
        if let Err(err) = worker.restore_state().await {
            
        }
    }
    
    async fn restore_state(&mut self) -> Result<()> {
        if let Some(path) = &self.state_file {
            if path.exists() {
                // 读取配置
                let data = tokio::fs::read_to_string(path).await?;
                let tasks: Vec<UploadTask> = serde_json::from_str(&data)?;
                
                for task in tasks {
                    // 恢复未完成的任务
                    match task.state {
                        UploadState::Queued | UploadState::Uploading | UploadState::Paused => {
                            let upload_id = task.id;
                            self.tasks.insert(upload_id, TaskHandle {
                                task,
                                cancellation_token: None,
                                join_handle: None,
                            });

                            // 将暂停和上传中的任务加入队列
                            if matches!(self.tasks[&upload_id].task.state, UploadState::Uploading | UploadState::Paused) {
                                self.tasks.get_mut(&upload_id).unwrap().task.state = UploadState::Queued;
                                self.queued_tasks.push(upload_id);
                            }
                        }
                        _ => {
                            // 保留已完成/失败/取消的任务记录
                            self.tasks.insert(task.id, TaskHandle {
                                task,
                                cancellation_token: None,
                                join_handle: None,
                            });
                        }
                    }
                }
            }
        }
        
        Ok(())
    }
}
