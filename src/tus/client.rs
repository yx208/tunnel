use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;
use reqwest::{Client, StatusCode, Url};
use reqwest::header::{HeaderMap, HeaderValue};
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use crate::config::get_config;
use crate::tus::constant::TUS_RESUMABLE;
use super::TusError;

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

    pub fn get_file_metadata(file_path: &Path) -> Result<String, TusError> {
        let filename = file_path.file_name()
            .ok_or(TusError::NotFileName)?
            .to_str()
            .ok_or(TusError::InvalidFileName)?;
        let encoded_filename = BASE64_STANDARD.encode(filename);

        Ok(format!("filename {}", encoded_filename))
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

    pub(crate) async fn create_upload(&self, file_size: u64, metadata: Option<&str>) -> Result<String, TusError> {
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
            return Err(TusError::UnexpectedStatus {
                status,
                message: String::from("Failed to create upload"),
            });
        }

        let location = match response_headers.get("Location") {
            Some(location) => {
                location
                    .to_str()
                    .map_err(|err| TusError::Other(err.into()))?
                    .to_string()
            },
            None => return Err(TusError::NotLocation)
        };

        if location.starts_with("http") {
            Ok(location)
        } else {
            let url = Url::parse(&self.endpoint)?;
            let origin = url.origin().ascii_serialization();
            Ok(format!("{}{}", origin, location))
        }
    }

    async fn get_upload_offset(&self, upload_url: &str) -> Result<u64, TusError> {
        let headers = TusClient::create_headers();
        let response = self
            .client
            .head(upload_url)
            .headers(headers)
            .send()
            .await?;

        if response.status() != StatusCode::OK {
            return Err(TusError::UnexpectedStatus {
                status: response.status(),
                message: String::from("Failed to get upload offset"),
            });
        }

        let offset = match response.headers().get("Upload-Offset") {
            Some(offset) => {
                offset
                    .to_str()
                    .map_err(|err| TusError::Other(err.into()))?
                    .parse::<u64>()
                    .map_err(|err| TusError::Other(err.into()))?
            },
            None => return Err(TusError::NotOffset)
        };

        Ok(offset)
    }

    async fn upload_chunk(&self, upload_url: &str, file: &mut File, offset: u64) -> Result<u64, TusError> {
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
            return Err(TusError::UnexpectedStatus {
                status: response.status(),
                message: String::from("Failed to upload chunk"),
            });
        }

        let new_offset = match response.headers().get("Upload-Offset") {
            Some(offset) => {
                offset
                    .to_str()
                    .map_err(|err| TusError::Other(err.into()))?
                    .parse::<u64>()
                    .map_err(|err| TusError::Other(err.into()))?
            },
            None => return Err(TusError::NotOffset)
        };

        Ok(new_offset)
    }

    pub async fn upload_file(&self, file_path: &str) -> Result<String, TusError> {
        let path = Path::new(file_path);
        let mut file = File::open(path)?;

        let file_size = file.metadata()?.len();
        let upload_url = self.create_upload(file_size, None).await?;

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
