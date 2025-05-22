use tunnel::config::get_config;
use tunnel::tus::types::TusClient;

#[tokio::main]
async fn main() {
    let config = get_config();
    let client = TusClient::new(&config.endpoint, 1024 * 1024 * 4);
    client.upload_file(&config.file_path, None).await.unwrap();
}
