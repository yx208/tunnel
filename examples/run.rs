use tokio::join;
use tunnel::{TransferConfig, TransferManager, TusTaskBuilder};
use tunnel::config::get_config;

#[tokio::main]
async fn main() {
    let config = get_config();
    let transfer_config = TransferConfig { concurrent: 2 };
    let manager_handle = TransferManager::new(transfer_config);
    let _ = manager_handle
        .manager
        .add(Box::new(TusTaskBuilder::new(
            config.endpoint.clone(),
            config.file_path.clone()
        )))
        .await;
    let _ = manager_handle
        .manager
        .add(Box::new(TusTaskBuilder::new(
            config.endpoint.clone(),
            config.file_path.clone()
        )))
        .await;
    let _ = manager_handle
        .manager
        .add(Box::new(TusTaskBuilder::new(
            config.endpoint.clone(),
            config.file_path.clone()
        )))
        .await;

    let _ = join!(manager_handle.handle);
}
