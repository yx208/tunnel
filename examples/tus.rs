use std::collections::HashMap;
use tokio::sync::broadcast;
use tunnel::config::get_config;
use tunnel::{Result, TransferConfig, TransferEvent, TransferProtocolBuilder};
use tunnel::scheduler::TunnelScheduler;
use tunnel::tus::{TusProtocolBuilder};

#[tokio::main]
async fn main() -> Result<()> {
    let scheduler = TunnelScheduler::new();
    scheduler.add_task(Box::new(create_tus_builder())).await?;
    scheduler.add_task(Box::new(create_tus_builder())).await?;
    scheduler.add_task(Box::new(create_tus_builder())).await?;

    tokio::spawn(handle_transfer_event(scheduler.subscribe()));

    run().await;
    
    Ok(())
}

fn create_tus_builder() -> impl TransferProtocolBuilder {
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
                if updates.len() > 0 {
                    for item in updates {
                        println!(
                            "{:.2?}MB/s, current: {:.2?}, Total: {:.2?}",
                            item.1.instant_speed / 1024.0 / 1024.0,
                            item.1.bytes_transferred as f64 / 1024.0 / 1024.0,
                            item.1.total_bytes as f64 / 1024.0 / 1024.0
                        );
                    }
                } else {
                    println!("No stats");
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
