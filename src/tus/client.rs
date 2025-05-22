use reqwest::{Client, StatusCode};
use reqwest::header::HeaderValue;
use super::types::{TusClient, UploadStrategy};
use super::errors::{Result, TusError};

impl TusClient {
    pub fn new(endpoint: &str, chunk_size: usize) -> Self {
        Self {
            client: Client::new(),
            chunk_size,
            endpoint: endpoint.to_string(),
            strategy: UploadStrategy::Chunked,
            speed_limit: None
        }
    }

    pub fn with_strategy(endpoint: &str, chunk_size: usize, strategy: UploadStrategy) -> Self {
        Self {
            client: Client::new(),
            endpoint: endpoint.to_string(),
            chunk_size,
            strategy,
            speed_limit: None,
        }
    }

    pub async fn create_upload(&self, file_size: u64, metadata: Option<&str>) -> Result<String> {
        let mut headers = TusClient::create_headers();
        headers.insert("Upload-Length", HeaderValue::from_str(&file_size.to_string())?);

        // 添加元数据如果有提供
        if let Some(meta) = metadata {
            headers.insert("Upload-Metadata", HeaderValue::from_str(meta)?);
        }

        let response = self
            .client
            .post(&self.endpoint)
            .headers(headers)
            .send()
            .await?;

        if response.status() != StatusCode::CREATED {
            return Err(TusError::ProtocolError("".to_string()));
        }

        // 从响应中获取上传地址
        let location = match response.headers().get("location") {
            Some(loc) => loc.to_str()?.to_string(),
            None => return Err(TusError::ProtocolError("Not 'location' response header".to_string())),
        };

        Ok(location)
    }
}