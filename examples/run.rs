use tokio::join;
use tunnel::{TransferConfig, TransferManager, TusTaskBuilder};
use tunnel::config::get_config;

#[tokio::main]
async fn main() {
    let system_config = get_config();
    let builder = TusTaskBuilder::new(system_config.endpoint, system_config.file_path);
    let config = TransferConfig::default();
    let manager_handle = TransferManager::new(config);
    let result = manager_handle
        .manager
        .add(Box::new(builder))
        .await;

    let _ = join!(manager_handle.handle);
}

