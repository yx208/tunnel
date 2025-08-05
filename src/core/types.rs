use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct TransferId(Uuid);

impl TransferId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

pub struct TaskBuildOptions {

}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub enum ProtocolType {
    Tus,
    Http
}

pub enum TransferMode {
    Streaming,
    Chunk,
}

pub struct TransferConfig {
    pub concurrent: usize,
}

impl Default for TransferConfig {
    fn default() -> Self {
        Self { concurrent: 3 }
    }
}
