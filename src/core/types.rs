use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct TransferId(Uuid);

impl TransferId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

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

