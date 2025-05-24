use std::path::PathBuf;
use tokio::sync::mpsc;
use super::types::TusClient;
use super::upload_manager::{ManagerCommand, UploadEvent};

pub struct UploadManagerWorker {

}

impl UploadManagerWorker {
    pub async fn run(
        client: TusClient,
        concurrent: usize,
        state_file: Option<PathBuf>,
        command_rx: mpsc::Receiver<ManagerCommand>,
        event_tx: mpsc::UnboundedSender<UploadEvent>
    ) {
        
    }
}
