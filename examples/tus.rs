use tokio::join;
use tunnel::{TransferConfig, TransferManager};

#[tokio::main]
async fn main() {
    let config = TransferConfig::default();
    let manager_handle = TransferManager::new(config);

    /*
    let event_rx = manager_handle.manager.subscribe();
    let event_handle = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            match event {
                TransferEvent::Progress { stats } => {
                    handle ...
                }
            }
        }
    });

    let upload_config = TusUploadConfig::new();
    let task = tus_task_builder(upload_config);
    manager_handle.add_task(task);

     */

    let handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
    let _ = join!(handle);
}
