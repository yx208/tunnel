use crate::config::get_config;
use crate::tus::client::TusClient;
use crate::tus::TusError;

mod tus;
mod config;

#[tokio::main]
async fn main() -> Result<(), TusError> {
    let config = get_config();
    let client = TusClient::new(&config.endpoint, 1024 * 1024 * 10);
    let result = client.upload_file(&config.file_path).await?;

    println!("{}", result);

    Ok(())
}
