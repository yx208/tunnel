use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

use tunnel::config::get_config;
use tunnel::{Result, TransferConfig, TransferProtocolBuilder, TransferTask};
use tunnel::progress::{ProgressAggregator};
use tunnel::tus::{TusProtocolBuilder};

#[tokio::main]
async fn main() -> Result<()> {
    let aggregator = create_aggregator();

    let mut task = TransferTask::new();
    let progress_tx = aggregator.registry_task(task.id.clone()).await;
    task.start(create_tus_builder(), Some(progress_tx)).await?;

    if task.handle.is_some() {
        println!("task is already running");
        let _ = tokio::join!(task.handle.unwrap());
        println!("task is already completed");
    }

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
