use tokio::sync::mpsc;

use crate::{TransferId, TransferProtocolBuilder};
use super::errors::Result;

pub struct TaskWorker {
    pub id: TransferId,
    pub handle: Option<tokio::task::JoinHandle<Result<()>>>,
}

impl TaskWorker {
    pub fn new() -> Self {
        Self {
            id: TransferId::new(),
            handle: None,
        }
    }

    pub async fn start(
        &mut self,
        builder: impl TransferProtocolBuilder,
        progress_tx: Option<mpsc::UnboundedSender<u64>>
    ) -> Result<()> {
        let mut context = builder.build_context().await;
        let protocol = builder.build_protocol().await;

        protocol.initialize(&mut context).await?;
        let handle = tokio::spawn(async move {
            protocol.execute(&context, progress_tx).await
        });

        self.handle = Some(handle);

        Ok(())
    }
}
