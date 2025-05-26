#![allow(warnings, warnings)]

use std::path::PathBuf;
use tunnel::config::get_config;
use tunnel::tus::client::{TusClient, UploadStrategy};
use tunnel::tus::errors::Result;
use tunnel::tus::manager::{UploadEvent, UploadManager};

#[tokio::main]
async fn main() -> Result<()> {
    let config = get_config();
    let client = TusClient::with_strategy(
        &config.endpoint,
        1024 * 1024 * 10,
        UploadStrategy::Streaming
    );

    let manager = UploadManager::new(
        client,
        3,
        Some("upload_state.json".into())
    );

    let file_path = PathBuf::from(&config.file_path);
    let id = manager.add_upload(file_path, None).await?;

    let join_handle = tokio::spawn(async move {
        let mut events = manager.subscribe_events().await;
        while let Some(event) = events.recv().await {
            match event {
                UploadEvent::Progress(progress) => {
                    println!(
                        "Upload {:?}: {:.1}% ({:.2} MB/s)",
                        progress.upload_id,
                        progress.percentage,
                        progress.instant_speed / (1024.0 * 1024.0)
                    );
                }
                UploadEvent::StateChanged { upload_id, old_state, new_state } => {
                    println!("Upload {:?}: {:?} -> {:?}", upload_id, old_state, new_state);
                }
                UploadEvent::Completed { upload_id, upload_url } => {
                    println!("Upload {:?} completed: {}", upload_id, upload_url);
                }
                UploadEvent::Failed { upload_id, error } => {
                    println!("Upload {:?} failed: {}", upload_id, error);
                }
            }
        }
    });

    tokio::join!(join_handle);

    Ok(())
}
