use tokio::sync::{broadcast, mpsc, oneshot};
use crate::progress::ProgressAggregator;
use crate::{TransferEvent, TransferId};
use super::task::{
    ManagerCommand,
    MangerWorkerHandle,
};
use super::{
    Result,
    TransferError,
    TransferProtocolBuilder,
};

pub struct TunnelScheduler {
    command_tx: mpsc::Sender<ManagerCommand>,
    worker: MangerWorkerHandle,
    aggregator: ProgressAggregator,
    event_tx: broadcast::Sender<TransferEvent>,
}

impl TunnelScheduler {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(128);
        let (command_tx, command_rx) = mpsc::channel(64);

        let worker = MangerWorkerHandle::new(command_rx);

        let aggregator = ProgressAggregator::new(event_tx.clone())
            .enable_report();

        Self {
            worker,
            aggregator,
            command_tx,
            event_tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TransferEvent> {
        self.event_tx.subscribe()
    }

    pub async fn add_task(&self, builder: Box<dyn TransferProtocolBuilder>) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let (bytes_tx, bytes_rx) = mpsc::unbounded_channel();

        self.command_tx
            .send(ManagerCommand::AddTask { builder, reply: reply_tx, bytes_tx })
            .await
            .map_err(|_| TransferError::ManagerShutdown)?;

        let id = reply_rx
            .await
            .map_err(|_| TransferError::ManagerShutdown)?;

        self.aggregator.registry_task(id, bytes_rx).await;

        Ok(())
    }

    pub async fn cancel_task(&self, id: TransferId) -> Result<()> {
        self.aggregator.unregister_task(&id).await;
        Ok(())
    }
}
