use std::collections::HashMap;
use tokio::sync::broadcast;
use tunnel::config::get_config;
use tunnel::{
    Result,
    TransferConfig,
    TransferEvent,
    TransferProtocolBuilder,
    TunnelScheduler,
    tus::TusProtocolBuilder
};

#[tokio::main]
async fn main() -> Result<()> {
    let scheduler = TunnelScheduler::new();
    scheduler.add_task(Box::new(create_task_builder())).await?;
    scheduler.add_task(Box::new(create_task_builder())).await?;
    scheduler.add_task(Box::new(create_task_builder())).await?;

    tokio::spawn(handle_transfer_event(scheduler.subscribe()));

    run().await;
    
    Ok(())
}

fn create_task_builder() -> impl TransferProtocolBuilder {
    let config = get_config();
    let mut headers = HashMap::new();
    headers.insert("Authorization".to_string(), config.token);

    let transfer_config = TransferConfig {
        headers,
        source: config.file_path,
    };
    let builder = TusProtocolBuilder::new(transfer_config, config.endpoint);

    builder
}

async fn handle_transfer_event(mut event_tx: broadcast::Receiver<TransferEvent>) {
    while let Ok(event) = event_tx.recv().await {
        match event {
            TransferEvent::Progress { updates } => {
                let sum_speed = updates
                    .iter()
                    .map(|(_, x)| x.instant_speed)
                    .sum::<f64>();
                println!("Current speed: {:.2?}MB/s", sum_speed  / 1024.0 / 1024.0);
            }
            TransferEvent::Success { id } => {
                println!("Task {:?} has been completed", id);
            }
            _ => {}
        }
    }
}

async fn run() {
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    }
}
