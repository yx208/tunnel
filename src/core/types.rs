use std::collections::HashMap;
use std::time::{Duration, Instant};
use serde_json::ser::State;
use uuid::Uuid;
use crate::TransferTask;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct TransferId(Uuid);

impl TransferId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Clone)]
pub struct TransferConfig {
    pub source: String,
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct TransferContext {
    pub destination: String,
    pub source: String,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
}

impl TransferContext {
    pub fn new(source: String) -> Self {
        Self {
            source,
            destination: String::new(),
            total_bytes: 0,
            transferred_bytes: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransferState {
    Queued,
    Preparing,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl TransferState {
    pub fn can_to(&self, state: &TransferState) -> bool {
        match (self, state)  {
            (TransferState::Queued, _) => true,

            (TransferState::Preparing, TransferState::Queued) => false,
            (TransferState::Preparing, _) => true,

            (TransferState::Running, _) => true,

            _ => false
        }
    }
}

#[derive(Debug, Clone)]
pub enum TransferEvent {
    /// Transfer state has changed
    StateChanged {
        id: TransferId,
        old_state: TransferState,
        new_state: TransferState,
    },

    /// Transfer progress has been updated
    Progress {
        updates: Vec<(TransferId, TransferStats)>,
    },

    /// Transfer task has started
    Started {
        id: TransferId,
    },

    /// Single transfer task completed successfully
    Success {
        id: TransferId,
    },

    /// Transfer task has failed
    Failed {
        id: TransferId,
        reason: String,
    },

    /// Transfer task has been cancelled
    Cancelled {
        id: TransferId,
    },

    /// Transfer task is being retried
    Retry {
        id: TransferId,
        attempt: u32,
        reason: String,
    },

    /// All transfer tasks have finished
    Finished,
}

#[derive(Clone, Debug)]
pub struct TransferStats {
    pub start_time: Instant,
    pub end_time: Option<Instant>,
    pub bytes_transferred: u64,
    pub total_bytes: u64,
    pub instant_speed: f64,
    pub average_speed: f64,
    pub eta: Option<Duration>,
}

