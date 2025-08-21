use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

use tunnel::config::get_config;
use tunnel::{Result, TransferConfig, TransferProtocolBuilder};
use tunnel::progress::{ProgressAggregator};
use tunnel::scheduler::TunnelScheduler;
use tunnel::tus::{TusProtocolBuilder};

#[tokio::main]
async fn main() -> Result<()> {
    let scheduler = TunnelScheduler::new();
    scheduler.add_task(Box::new(create_tus_builder())).await?;
    scheduler.add_task(Box::new(create_tus_builder())).await?;
    scheduler.add_task(Box::new(create_tus_builder())).await?;

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

fn create_aggregator() -> ProgressAggregator {
    let cancellation_token = CancellationToken::new();
    let aggregator = ProgressAggregator::new(cancellation_token.clone(), true);
    
    let mut progress_receiver = aggregator.subscribe();
    tokio::spawn(async move {
        while let Ok(stats_vec) = progress_receiver.recv().await {
            if stats_vec.len() > 0 {
                for item in stats_vec {
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
    });

    aggregator
}

async fn run() {
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    }
}
