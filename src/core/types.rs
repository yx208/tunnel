use std::collections::HashMap;
use serde_json::ser::State;
use uuid::Uuid;

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
    StateChanged {
        id: TransferId,
        old_state: TransferState,
        new_state: TransferState,
    },

    Started {
        id: TransferId,
    },

    Completed {
        id: TransferId,
    },

    Failed {
        id: TransferId,
        reason: String,
    },

    Cancelled {
        id: TransferId,
    }
}

