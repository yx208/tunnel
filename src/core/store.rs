use std::collections::{HashMap, VecDeque};
use std::time::Instant;
use tokio::task::JoinHandle;
use crate::{TransferId, TransferState, TransferTask};

#[derive(Default)]
pub struct TaskStore {
    tasks: HashMap<TransferId, TransferTask>,
    pending_queue: VecDeque<TransferId>,
    running_tasks: HashMap<TransferId, JoinHandle<()>>,
    completed_tasks: Vec<TransferId>,
    failed_tasks: HashMap<TransferId, String>,
}

impl TaskStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_task(&mut self, task: TransferTask) {
        let id = task.id.clone();
        self.tasks.insert(id.clone(), task);
        self.pending_queue.push_back(id);
    }

    pub fn has_next_task(&self) -> bool {
        self.pending_queue.len() > 0
    }

    pub fn get_next_pending(&mut self) -> Option<TransferId> {
        self.pending_queue.pop_front()
    }

    pub fn update_state(&mut self, id: &TransferId, state: TransferState) {
        if let Some(task) = self.tasks.get_mut(id) {
            task.state = state.clone();

            match state {
                TransferState::Running => {
                    task.started_at = Some(Instant::now());
                }
                TransferState::Completed | TransferState::Failed | TransferState::Cancelled => {
                    task.completed_at = Some(Instant::now());
                }
                _ => {}
            }
        }
    }

    pub fn mark_completed(&mut self, id: &TransferId) {
        self.running_tasks.remove(id);
        self.update_state(id, TransferState::Completed);
        self.completed_tasks.push(id.clone());
    }

    pub fn mark_failed(&mut self, id: &TransferId, reason: String) {
        self.running_tasks.remove(id);
        self.update_state(id, TransferState::Failed);
        self.failed_tasks.insert(id.clone(), reason);
    }

    pub fn cancel_task(&mut self, id: &TransferId) {
        if let Some(task) = self.tasks.remove(id) {
            match task.state {
                TransferState::Queued => {
                    self.pending_queue.retain(|x| x != id);
                }
                TransferState::Running => {
                    if let Some(handle) = self.running_tasks.remove(id) {
                        handle.abort();
                    }
                }
                TransferState::Completed => {
                    self.completed_tasks.retain(|x| x != id);
                }
                TransferState::Failed => {
                    self.failed_tasks.remove(id);
                }
                TransferState::Paused => {}
                TransferState::Cancelled => {}
            }
        }
    }

    pub fn add_running(&mut self, id: TransferId, handle: JoinHandle<()>) {
        self.update_state(&id, TransferState::Running);
        self.running_tasks.insert(id, handle);
    }

    pub fn get_task(&self, id: &TransferId) -> Option<&TransferTask> {
        self.tasks.get(id)
    }
}
