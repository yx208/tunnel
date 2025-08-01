use tokio::join;
use tunnel::TransferManager;

#[tokio::main]
async fn main() {
    let manager = TransferManager::new();

    let handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
    let _ = join!(handle);
}
