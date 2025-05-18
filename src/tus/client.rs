use std::error::Error;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;
use reqwest::{Client, StatusCode, Url};
use reqwest::header::{HeaderMap, HeaderValue};
use crate::config::get_config;
use crate::tus::constant::TUS_RESUMABLE;

pub struct TusClient {
    client: Client,
    endpoint: String,
    chunk_size: usize,
}

impl TusClient {
    pub fn create_headers() -> HeaderMap {
        let config = get_config();
        let mut headers = HeaderMap::new();
        headers.insert("Tus-Resumable", HeaderValue::from_static(TUS_RESUMABLE));
        headers.insert("Authorization", HeaderValue::from_str(&config.token).unwrap());

        headers
    }
}

impl TusClient {
    pub fn new(endpoint: &str, chunk_size: usize) -> Self {
        Self {
            client: Client::new(),
            endpoint: endpoint.to_string(),
            chunk_size,
        }
    }

    pub(crate) async fn create_upload(&self, file_size: u64, metadata: Option<&str>) -> Result<String, Box<dyn Error>> {
        let mut headers = TusClient::create_headers();
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

        if location.starts_with("http") {
            Ok(location)
        } else {
            let url = Url::parse(&self.endpoint)?;
            let origin = url.origin().ascii_serialization();
            Ok(format!("{}{}", origin, location))
        }
    }

    async fn get_upload_offset(&self, upload_url: &str) -> Result<u64, Box<dyn Error>> {
        let headers = TusClient::create_headers();
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

    async fn upload_chunk(&self, upload_url: &str, file: &mut File, offset: u64) -> Result<u64, Box<dyn Error>> {
        let mut buffer = vec![0; self.chunk_size];
        file.seek(SeekFrom::Start(offset))?;
        let bytes_read = file.read(&mut buffer)?;

        // file end
        if bytes_read == 0 {
            return Ok(offset);
        }

        buffer.truncate(bytes_read);

        let mut headers = TusClient::create_headers();
        headers.insert("Upload-Offset", HeaderValue::from_str(&offset.to_string())?);
        headers.insert("Content-Type", HeaderValue::from_static("application/offset+octet-stream"));

        let response = self
            .client
            .patch(upload_url)
            .headers(headers)
            .body(buffer)
            .send()
            .await?;

        if response.status() != StatusCode::NO_CONTENT {
            return Err(format!("Failed to upload chunk: {}", response.status()).into());
        }

        let new_offset = match response.headers().get("Upload-Offset") {
            Some(new_offset) => new_offset.to_str()?.parse::<u64>()?,
            None => {
                return Err("No upload offset returned from server".into());
            }
        };

        Ok(new_offset)
    }

    pub async fn upload_file(&self, file_path: &str, metadata: Option<&str>) -> Result<String, Box<dyn Error>> {
        let path = Path::new(file_path);
        let mut file = File::open(path)?;

        let file_size = file.metadata()?.len();
        let upload_url = self.create_upload(file_size, metadata).await?;

        let mut offset = self.get_upload_offset(&upload_url).await?;
        
        while offset < file_size {
            offset = self.upload_chunk(&upload_url, &mut file, offset).await?;
            
            tokio::time::sleep(Duration::from_secs(1)).await;
            
            println!("Uploaded: {}/{} bytes ({}%)",
                     offset,
                     file_size,
                     (offset as f64 / file_size as f64 * 100.0) as u64
            );
        }

        Ok(upload_url)
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
