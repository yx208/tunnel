use std::error::Error;
use reqwest::{Client, StatusCode};
use reqwest::header::{HeaderMap, HeaderValue};
use crate::tus::constant::TUS_RESUMABLE;

pub struct TusClient {
    client: Client,
    endpoint: String,
    chunk_size: usize,
}

impl TusClient {
    pub fn new(endpoint: &str, chunk_size: usize) -> Self {
        Self {
            client: Client::new(),
            endpoint: endpoint.to_string(),
            chunk_size,
        }
    }

    async fn create_upload(&self, file_size: u64, metadata: Option<&str>) -> Result<String, Box<dyn Error>> {
        let mut headers = HeaderMap::new();
        headers.insert("Tus-Resumable", HeaderValue::from_static(TUS_RESUMABLE));
        headers.insert("Authorization", HeaderValue::from_static("tus-token"));
        headers.insert("Upload-Length", HeaderValue::from_str(&file_size.to_string())?);

        // Has metadata
        if let Some(metadata) = metadata {
            headers.insert("Upload-Metadata", HeaderValue::from_str(metadata)?);
        }

        let response = self
            .client
            .post(&self.endpoint)
            .headers(headers)
            .send()
            .await?;

        let status = response.status();
        let response_headers = response.headers().clone();

        if status != StatusCode::CREATED {
            return Err(format!("Failed to create upload: {}", status).into());
        }

        let location = match response_headers.get("Location") {
            Some(location) => location.to_str()?.to_string(),
            None => {
                return Err("No upload location returned from server".into());
            }
        };

        Ok(location)
    }

    async fn get_upload_offset(&self, upload_url: &str) -> Result<u64, Box<dyn Error>> {
        let mut headers = HeaderMap::new();
        headers.insert("Tus-Resumable", HeaderValue::from_static(TUS_RESUMABLE));

        let response = self
            .client
            .head(upload_url)
            .headers(headers)
            .send()
            .await?;

        if response.status() != StatusCode::OK {
            return Err(format!("Failed to get upload offset: {}", response.status()).into());
        }

        let offset = match response.headers().get("Upload-Offset") {
            Some(offset)  => offset.to_str()?.parse::<u64>()?,
            None => {
                return Err("No upload offset returned from server".into());
            }
        };

        Ok(offset)
    }
}

mod tests {
    use super::*;
    use crate::config::get_config;

    #[tokio::test]
    async fn should_be_run() {
        let config = get_config();
        let chunk_size = 1024 * 1024 * 1;
        let client = TusClient::new(&config.endpoint, chunk_size);

        let file_size = 1024 * 1024 * 2;
        let result = client.create_upload(file_size, None).await;
        assert!(result.is_ok());
    }
}
