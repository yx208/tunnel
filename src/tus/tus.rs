use std::collections::HashMap;
use std::str::FromStr;
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::io::ReaderStream;
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use url::Url;

use crate::core::{
    Result,
    TransferError,
    TransferContext,
    TransferProtocolBuilder,
};
use crate::progress::FileStream;
use crate::{TransferConfig, TransferProtocol};

#[derive(Debug, Clone)]
pub struct TusConfig {
    pub endpoint: String,
    pub buffer_size: usize,
    pub headers: HashMap<String, String>,
}

impl TusConfig {
    pub fn new(endpoint: String) -> Self {
        Self {
            endpoint,
            buffer_size: 1024 * 1024 * 2,
            headers: HashMap::new(),
        }
    }
}

pub struct TusProtocol {
    config: TusConfig,
    client: Client,
}

impl TusProtocol {
    pub fn new(config: TusConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }

    async fn create_upload(&self, length: u64) -> Result<String> {
        let headers = self.create_header_map()?;
        let response = self.client
            .post(&self.config.endpoint)
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
            let base_url = Url::parse(&self.config.endpoint)?;
            let final_url = base_url.join(location)?;
            Ok(final_url.to_string())
        }
    }

    fn create_header_map(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        for (k, v) in &self.config.headers {
            headers.insert(
                HeaderName::from_str(&k)?,
                HeaderValue::from_str(&v)?
            );
        }

        Ok(headers)
    }

    async fn get_upload_offset(&self, upload_url: &str) -> Result<u64> {
        let headers = self.create_header_map()?;
        let response = self.client
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
}

#[async_trait]
impl TransferProtocol for TusProtocol {
    async fn initialize(&self, context: &mut TransferContext) -> Result<()> {
        let file_size = std::fs::metadata(&context.source)?.len();
        let upload_url = self.create_upload(file_size).await?;
        let offset = self.get_upload_offset(&upload_url).await?;

        context.destination = upload_url;
        context.transferred_bytes = offset;
        context.total_bytes = file_size;

        Ok(())
    }

    async fn execute(
        &self,
        ctx: &TransferContext,
        progress_tx: Option<mpsc::UnboundedSender<u64>>
    ) -> Result<()> {
        let mut headers = HeaderMap::new();
        for (k, v) in &self.config.headers {
            headers.insert(
                HeaderName::from_str(&k)?,
                HeaderValue::from_str(&v)?
            );
        }

        let file = tokio::fs::File::open(&ctx.source).await?;
        let reader_stream = ReaderStream::new(file);
        let stream = reqwest::Body::wrap_stream(FileStream::new(reader_stream, progress_tx));
        let response = self.client
            .patch(&ctx.destination)
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

#[derive(Clone)]
pub struct TusProtocolBuilder {
    config: TransferConfig,
    endpoint: String,
}

impl TusProtocolBuilder {
    pub fn new(config: TransferConfig, endpoint: String) -> Self {
        Self { config, endpoint }
    }
}

#[async_trait]
impl TransferProtocolBuilder for TusProtocolBuilder {
    async fn build_context(&self) -> TransferContext {
        TransferContext::new(self.config.source.clone())
    }

    async fn build_protocol(&self) -> Box<dyn TransferProtocol> {
        let config = TusConfig {
            endpoint: self.endpoint.clone(),
            headers: self.config.headers.clone(),
            buffer_size: 1024 * 1024 * 2,
        };
        Box::new(TusProtocol::new(config))
    }
}

