use std::collections::HashMap;
use reqwest::Client;
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
async fn main() {
    test_upload().await;
}

async fn build_context(config: &TusConfig, size: u64) -> Result<TransferContext> {
    let client = Client::new();
    let upload_url = TusProtocol::create_upload(client.clone(), config.clone(), size).await?;
    let offset = TusProtocol::get_upload_offset(client, config.clone(), &upload_url).await?;

    Ok(TransferContext {
        destination: upload_url,
        source: "".to_string(),
        total_bytes: size,
        transferred_bytes: offset,
    })
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

async fn test_upload() {
    let local_config = get_config();
    let tus_config = create_tus_config();

    let file = tokio::fs::File::open(local_config.file_path).await.unwrap();
    let metadata = file.metadata().await.unwrap();
    let context = build_context(&tus_config, metadata.len()).await.unwrap();

    let cancellation_token = CancellationToken::new();
    let aggregator = ProgressAggregator::new(cancellation_token.clone(), true);
    let mut progress_receiver = aggregator.subscribe();
    tokio::spawn(async move {
        while let Ok(stats_vec) = progress_receiver.recv().await {
            println!("{:#?}", stats_vec);
        }
    });

    let id = TransferId::new();
    let sender = aggregator.registry_task(id.clone()).await;

    let protocol = TusProtocol::new(tus_config);
    match protocol.stream_upload(file, context, Some(sender)).await {
        Ok(_) => {
            println!("Stream upload successful");
        }
        Err(e) => {
            println!("Upload error: {:?}", e);
        },
    }
}
