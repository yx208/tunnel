use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use url::Url;
use reqwest::header::{HeaderValue, HeaderMap};
use reqwest::{Client, Request, Response, StatusCode};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use crate::config::get_config;
use super::errors::{Result, TusError};

/// 请求钩子 trait
pub trait RequestHook: Sync + Send {
    fn before_request(&self, request: &mut Request) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UploadStrategy {
    /// Not support
    Chunked,

    /// Default strategy
    Streaming
}

#[derive(Clone)]
pub struct TusClient {
    pub client: Client,
    pub endpoint: String,
    pub buffer_size: usize,
    pub strategy: UploadStrategy,
    pub speed_limit: Option<u64>,
    pub headers: HashMap<String, String>,
    request_hook: Option<Arc<dyn RequestHook>>,
}

impl TusClient {
    pub fn new(endpoint: &str, buffer_size: usize) -> Self {
        Self {
            client: Client::new(),
            buffer_size,
            endpoint: endpoint.to_string(),
            strategy: UploadStrategy::Streaming,
            speed_limit: None,
            headers: HashMap::new(),
            request_hook: None,
        }
    }

    pub fn create_headers() -> HeaderMap {
        let config = get_config();
        let mut headers = HeaderMap::new();
        headers.insert("Tus-Resumable", HeaderValue::from_static("1.0.0"));
        headers.insert("Authorization", HeaderValue::from_str(&config.token).unwrap());
        headers.insert("mac-identifier", HeaderValue::from_static("media-web"));
        headers.insert("mac-organization", HeaderValue::from_static("3"));

        headers
    }

    pub fn parse_offset_header(status: u16, headers: &HeaderMap) -> Result<u64> {
        match headers.get("Upload-Offset") {
            Some(value) => {
                let offset =  value
                    .to_str()
                    .map_err(|err| TusError::ServerError {
                        status_code: status,
                        message: format!("Failed to parse header <Upload-Offset>: {}", err),
                    })?
                    .parse::<u64>()
                    .map_err(|err| TusError::ServerError {
                        status_code: status,
                        message: format!("Failed to parse header <Upload-Offset> to number: {}", err),
                    })?;

                Ok(offset)
            },
            None => Err(TusError::server_error(status, "No 'upload-offset' header in response"))
        }
    }

    pub fn with_hook(mut self, request_hook: impl RequestHook + 'static) -> Self {
        self.request_hook = Some(Arc::new(request_hook));
        self
    }

    /// 执行请求
    async fn execute(&self, mut request: Request) -> Result<Response> {
        if let Some(hook) = &self.request_hook {
            hook.before_request(&mut request)?;
        }

        Ok(self.client.execute(request).await?)
    }

    pub async fn create_upload(&self, file_size: u64, metadata: Option<HashMap<String, String>>) -> Result<String> {
        let mut request = self.client
            .post(&self.endpoint)
            .header("Tus-Resumable", "1.0.0")
            .header("Upload-Length", file_size);

        // Insert metadata header
        if let Some(meta) = metadata {
            let m = encode_metadata(&meta);
            request = request.header(
                "Upload-Metadata",
                HeaderValue::from_str(&m)?,
            );
        }
        
        let request = request.build()?;
        let response = self.execute(request).await?;

        if response.status() != StatusCode::CREATED {
            return Err(TusError::server_error(response.status().as_u16(), "Failed to create upload"));
        }

        let location = match response.headers().get("location") {
            Some(location) => {
                location.to_str()
                    .map_err(|err| TusError::ServerError {
                        status_code: response.status().as_u16(),
                        message: format!("Failed to parse location header: {}", err),
                    })?
                    .to_string()
            },
            None => {
                return Err(TusError::server_error(
                    response.status().as_u16(),
                    "Not 'location' header in response",
                ));
            }
        };

        if location.starts_with("http") {
            Ok(location)
        } else {
            let url = Url::parse(&self.endpoint)
                .map_err(|_e| TusError::ParamError(format!("Invalid url: {:?}", self.endpoint)))?;
            let origin = url.origin().ascii_serialization();

            Ok(format!("{}{}", origin, location))
        }
    }

    pub async fn get_upload_offset(&self, upload_url: &str) -> Result<u64> {
        let request = self.client
            .head(upload_url)
            .header("Tus-Resumable", "1.0.0")
            .build()?;
        let response = self.execute(request).await?;

        let status = response.status();
        if response.status() != StatusCode::OK && response.status() != StatusCode::NO_CONTENT {
            return Err(TusError::server_error(response.status().as_u16(), "Failed to get upload offset"));
        }

        let offset = TusClient::parse_offset_header(status.as_u16(), response.headers())?;

        Ok(offset)
    }

    pub async fn upload_file_streaming(
        &self,
        upload_url: &str,
        file_size: u64,
        offset: u64,
        body: reqwest::Body,
    ) -> Result<String> {
        let remaining_size = file_size - offset;
        let request = self.client
            .patch(upload_url)
            .body(body)
            .header("Tus-Resumable", "1.0.0")
            .header("Content-Type", "application/offset+octet-stream")
            .header("Content-Length", remaining_size)
            .header("Upload-Offset", offset)
            .build()?;
        
        let response = self.execute(request).await?;

        // 验证响应
        let status = response.status();
        if status != StatusCode::NO_CONTENT {
            return Err(TusError::server_error(
                status.as_u16(),
                format!("Upload failed with status {}", status),
            ));
        }

        // 验证最终偏移量
        let final_offset = TusClient::parse_offset_header(status.as_u16(), response.headers())?;
        if final_offset != file_size {
            return Err(TusError::UploadIncomplete {
                expected: file_size,
                actual: final_offset,
            });
        }

        Ok(upload_url.to_string())
    }

    pub async fn cancel_upload(&self, upload_url: &str) -> Result<()> {
        let headers = TusClient::create_headers();

        let response = self
            .client
            .delete(upload_url)
            .headers(headers)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() && status != StatusCode::NOT_FOUND {
            return Err(TusError::server_error(status.as_u16(), "Failed to cancel upload"));
        }

        Ok(())
    }
}

fn encode_metadata(metadata: &HashMap<String, String>) -> String {
    metadata
        .iter()
        .map(|(k, v)| {
            let encoded_value = STANDARD.encode(v);
            format!("{}={}", k, encoded_value)
        })
        .collect::<Vec<_>>()
        .join(",")
}
