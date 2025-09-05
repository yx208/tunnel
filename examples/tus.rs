use std::collections::HashMap;
use tokio::sync::broadcast;
use tunnel::config::get_config;
use tunnel::{
    Result,
    TransferConfig,
    TransferEvent,
    TransferProtocolBuilder,
    Tunnel,
    protocol::TusProtocolBuilder,
};

#[tokio::main]
async fn main() -> Result<()> {
    let tunnel = Tunnel::new();

    tokio::spawn(handle_transfer_event(tunnel.subscribe()));

    let task1 = tunnel.add_task(Box::new(create_task_builder())).await?;
    tunnel.add_task(Box::new(create_task_builder())).await?;
    tunnel.add_task(Box::new(create_task_builder())).await?;

    let _ = tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(4)).await;
        let _ = tunnel.cancel_task(task1).await;
    }).await;

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
                println!("===================");
                for update in updates {
                    println!(
                        "Task {:?}: {:.2?}MB/s",
                        update.0,
                        update.1.instant_speed  / 1024.0 / 1024.0
                    );
                }
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
