use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, Semaphore};
use tokio::sync::mpsc;
use crate::core::types::TransferId;
use crate::{TransferTask};

pub struct TaskQueue {
    task_tx: mpsc::Sender<TransferTask>,
    tasks: Arc<RwLock<Vec<TransferTask>>>,
    semaphore: Arc<Semaphore>,
}

impl TaskQueue {
    pub fn new(semaphore: Arc<Semaphore>, task_tx: mpsc::Sender<TransferTask>) -> Self {
        Self {
            task_tx,
            semaphore,
            tasks: Arc::new(RwLock::new(Vec::new())),
        }
    }
    
    pub async fn push(&self, task: TransferTask) {
        if self.semaphore.available_permits() != 0 {
            self.task_tx.send(task.clone()).await.unwrap();
        }
        
        self.tasks.write().await.push(task);
    }
}

pub struct WorkerPool {
    semaphore: Arc<Semaphore>,
    task_handles: Arc<RwLock<HashMap<TransferId, tokio::task::JoinHandle<()>>>>,
}

impl WorkerPool {
    pub fn new(semaphore: Arc<Semaphore>) -> Self {
        Self {
            semaphore,
            task_handles: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn start(mut self, mut task_rx: mpsc::Receiver<TransferTask>) {
        loop {
            println!("Waiting for receive task");
            let receive_task = task_rx.recv().await;
            match receive_task {
                Some(task) => {
                    self.execute(task).await;
                }
                None => {
                    println!("Receive None task");
                    break;
                }
            }
        }
    }
    
    async fn execute(&self, task: TransferTask) {
        println!("Execute: {:?}", task.id);

        let task_id = task.id.clone();
        let semaphore = self.semaphore.clone();
        let handles = self.task_handles.clone();

        let handle = tokio::spawn(async move {
            let permit = semaphore.acquire().await;
            tokio::time::sleep(Duration::from_secs(1)).await;
            handles.write().await.remove(&task_id);
            drop(permit);
        });

        self.task_handles.write().await.insert(task.id, handle);
    }
}
