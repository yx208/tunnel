use tokio::sync::{broadcast, mpsc, oneshot};
use crate::progress::ProgressAggregator;
use crate::{TransferEvent, TransferId};
use super::task::{
    ManagerCommand,
    MangerWorker,
};
use super::{
    Result,
    TransferError,
    TransferProtocolBuilder,
};

pub struct TunnelScheduler {
    command_tx: mpsc::UnboundedSender<ManagerCommand>,
    event_tx: broadcast::Sender<TransferEvent>,
    worker: MangerWorker,
    aggregator: ProgressAggregator,
}

impl TunnelScheduler {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(128);
        let (command_tx, command_rx) = mpsc::unbounded_channel();

        let worker = MangerWorker::new(command_rx, event_tx.clone());

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

    pub async fn shutdown(&mut self) {
        self.worker.shutdown();
        self.aggregator.shutdown().await;
        self.event_tx.closed().await;
        self.command_tx.closed().await;
    }

    pub async fn add_task(&self, builder: Box<dyn TransferProtocolBuilder>) -> Result<TransferId> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let (bytes_tx, bytes_rx) = mpsc::unbounded_channel();

        self.command_tx
            .send(ManagerCommand::AddTask { builder, reply: reply_tx, bytes_tx })
            .map_err(|_| TransferError::ManagerShutdown)?;

        let id = reply_rx
            .await
            .map_err(|_| TransferError::ManagerShutdown)?;

        self.aggregator.registry_task(id.clone(), bytes_rx).await;

        Ok(id)
    }

    pub async fn cancel_task(&self, id: TransferId) -> Result<()> {
        self.aggregator.unregister_task(&id).await;

        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(ManagerCommand::CancelTask { id, reply: reply_tx })
            .map_err(|_| TransferError::ManagerShutdown)?;
        
        let _ = reply_rx.await.map_err(|_| TransferError::ManagerShutdown)?;

        Ok(())
    }

    pub async fn pause_task(&self, id: TransferId) -> Result<()> {
        self.aggregator.unregister_task(&id).await;
        let (reply_tx, reply_rx) = oneshot::channel();

        self.command_tx
            .send(ManagerCommand::PauseTask { id, reply: reply_tx })
            .map_err(|_| TransferError::ManagerShutdown)?;

        let _ = reply_rx.await.map_err(|_| TransferError::ManagerShutdown)?;

        Ok(())
    }

    pub async fn resume_task(&self, id: TransferId) -> Result<()> {
        Ok(())
    }
}
