use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

use tunnel::config::get_config;
use tunnel::{Result, TransferId};
use tunnel::progress::ProgressAggregator;
use tunnel::tus::{
    TransferContext,
    TusConfig,
    TusProtocol,
};

#[tokio::main]
async fn main() -> Result<()> {
    let local_config = get_config();
    let tus_config = create_tus_config();

    let mut context = TransferContext::new(local_config.file_path);
    let aggregator = create_aggregator();

    let id = TransferId::new();
    let sender = aggregator.registry_task(id.clone()).await;

    let protocol = TusProtocol::new(tus_config);
    protocol.initialize(&mut context).await?;
    match protocol.stream_upload(context, Some(sender)).await {
        Ok(_) => {
            aggregator.unregister_task(id).await;
            println!("Stream upload successful");
        }
        Err(e) => {
            println!("Upload error: {:?}", e);
        },
    }
    
    let handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    });
    
    let _ = tokio::join!(handle);

    Ok(())
}

fn create_tus_config() -> TusConfig {
    let config = get_config();
    let mut headers = HashMap::new();
    headers.insert("Authorization".to_string(), config.token);

    TusConfig {
        endpoint: config.endpoint.clone(),
        buffer_size: 1024 * 102 * 2,
        headers
    }
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
