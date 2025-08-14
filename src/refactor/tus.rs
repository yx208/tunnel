use std::collections::HashMap;
use std::str::FromStr;
use futures_util::Stream;
use tokio::sync::mpsc;
use tokio_util::io::ReaderStream;
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use url::Url;

use crate::core::{Result, TransferError};
use super::progress::FileStream;

#[derive(Debug, Clone)]
pub struct TusConfig {
    pub endpoint: String,
    pub buffer_size: usize,
    pub headers: HashMap<String, String>,
}

pub struct TusProtocol {
    config: TusConfig,
    client: Client,
    context: Option<TransferContext>,
}

impl TusProtocol {
    pub fn new(config: TusConfig) -> Self {
        Self {
            config,
            context: None,
            client: Client::new(),
        }
    }

    pub async fn create_upload(
        client: Client,
        config: TusConfig,
        length: u64
    ) -> Result<String> {
        let headers = TusProtocol::create_header_map(&config.headers)?;
        let response = client
            .post(&config.endpoint)
            .header("Tus-Resumable", "1.0.0")
            .header("Upload-Length", length)
            .headers(headers)
            .send()
            .await?;

        if response.status() != reqwest::StatusCode::CREATED {
            return Err(TransferError::protocol_specific(
                "TUS",
                format!("Failed to create upload: {}", response.status())
            ));
        }

        let location = response
            .headers()
            .get("Location")
            .ok_or_else(|| TransferError::protocol_specific(
                "TUS",
                "No Location header in response"
            ))?
            .to_str()
            .map_err(|err| TransferError::Other(err.to_string()))?;

        if location.starts_with("http") {
            Ok(location.to_string())
        } else {
            let base_url = Url::parse(&config.endpoint)?;
            let final_url = base_url.join(location)?;
            Ok(final_url.to_string())
        }
    }

    fn create_header_map(header_map: &HashMap<String, String>) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        for (k, v) in header_map {
            headers.insert(
                HeaderName::from_str(&k)?,
                HeaderValue::from_str(&v)?
            );
        }

        Ok(headers)
    }

    pub async fn get_upload_offset(
        client: Client,
        config: TusConfig,
        upload_url: &str
    ) -> Result<u64> {
        let headers = TusProtocol::create_header_map(&config.headers)?;
        let response = client
            .head(upload_url)
            .header("Tus-Resumable", "1.0.0")
            .headers(headers)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(TransferError::protocol_specific(
                "TUS",
                format!("Failed to get upload offset: {}", response.status())
            ));
        }

        let offset = response
            .headers()
            .get("Upload-Offset")
            .ok_or_else(|| TransferError::protocol_specific(
                "TUS",
                "No Upload-Offset header in response"
            ))?
            .to_str()
            .map_err(|err| TransferError::Other(err.to_string()))?
            .parse::<u64>()
            .map_err(|err| TransferError::protocol_specific(
                "TUS",
                format!("Invalid Upload-Offset: {}", err)
            ))?;

        Ok(offset)
    }

    pub async fn stream_upload(
        &self,
        file: tokio::fs::File,
        ctx: TransferContext,
        progress_tx: Option<mpsc::UnboundedSender<u64>>
    ) -> Result<()> {
        let mut headers = HeaderMap::new();
        for (k, v) in &self.config.headers {
            headers.insert(
                HeaderName::from_str(&k)?,
                HeaderValue::from_str(&v)?
            );
        }

        let reader_stream = ReaderStream::with_capacity(file, 1024 * 1024 * 2);
        let stream = reqwest::Body::wrap_stream(FileStream::new(reader_stream, progress_tx));
        let response = self.client
            .patch(ctx.destination)
            .headers(headers)
            .header("Tus-Resumable", "1.0.0")
            .header("Content-Type", "application/offset+octet-stream")
            .header("Content-Length", ctx.total_bytes)
            .header("Upload-Offset", ctx.transferred_bytes)
            .body(stream)
            .send()
            .await?;

        if !response.status().is_success() {
            println!("Request error: {}", response.status());
        }

        Ok(())
    }
}

pub struct TransferContext {
    pub destination: String,
    pub source: String,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
}
