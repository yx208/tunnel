use tunnel::config::get_config;
use tunnel::tus::types::TusClient;
use tunnel::tus::errors::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let config = get_config();
    let client = TusClient::new(&config.endpoint, 1024 * 1024 * 10);
    client.upload_file(&config.file_path, None).await?;
    
    Ok(())
}
