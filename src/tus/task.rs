use std::collections::HashMap;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use super::types::{UploadState, UploadId};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UploadTask {
    pub id: UploadId,
    pub file_path: PathBuf,
    pub file_size: u64,
    pub upload_url: Option<String>,
    pub state: UploadState,
    pub bytes_uploaded: u64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
}