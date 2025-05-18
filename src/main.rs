use std::error::Error;
use crate::config::get_config;
use crate::tus::client::TusClient;

mod tus;
mod config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = get_config();
    let metadata = format!("filename {},filetype dmlkZW8vbXA0", &config.file_name);
    let client = TusClient::new(&config.endpoint, 1024 * 1024 * 10);
    let result = client.upload_file(&config.file_path, Some(&metadata)).await?;

    println!("{}", result);
    
    Ok(())
}
