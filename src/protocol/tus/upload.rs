use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use futures::Stream;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, StatusCode};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use super::types::{TusConfig, TusMetadata, TusUploadOptions};
use crate::config::get_config;
use crate::core::{
    ProtocolType, Result, TransferChunk, TransferContext, TransferError, TransferId,
    TransferMetadata, TransferMode, TransferOptions, TransferProcessor, TransferProtocol,
    TransferTaskBuilder,
};

struct TusUploadProtocol {
    client: Client,
    config: TusConfig,
    options: TusUploadOptions,
}

impl TusUploadProtocol {
    pub fn new(config: TusConfig, options: TusUploadOptions) -> Result<Self> {
        let client = Client::builder().timeout(config.timeout).build()?;

        Ok(Self {
            client,
            config,
            options,
        })
    }

    async fn create_upload(&self, file_size: u64) -> Result<String> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Tus-Resumable",
            HeaderValue::from_str(&self.config.tus_version)?,
        );
        headers.insert("Upload-Length", HeaderValue::from(file_size));

        if !self.options.metadata.is_empty() {
            let encoded_metadata = self.encode_metadata(&self.options.metadata);
            headers.insert("Upload-Metadata", HeaderValue::from_str(&encoded_metadata)?);
        }

        if !self.config.headers.is_empty() {
            for (key, value) in &self.config.headers {
                headers.insert(
                    HeaderName::from_bytes(key.as_bytes())?,
                    HeaderValue::from_str(value)?,
                );
            }
        }

        let response = self
            .client
            .post(&self.config.endpoint)
            .headers(headers)
            .send()
            .await?;

        if response.status() != StatusCode::CREATED {
            return Err(TransferError::protocol_specific(
                "TUS",
                format!("Failed to create upload: {}", response.status()),
            ));
        }

        let location = response
            .headers()
            .get("Location")
            .ok_or_else(|| TransferError::protocol("No Location header in response"))?
            .to_str()
            .map_err(|err| {
                TransferError::Protocol(format!("Failed to parse Location header: {}", err))
            })?;

        let upload_url = if location.starts_with("http") {
            location.to_string()
        } else {
            let base_url = url::Url::parse(&self.config.endpoint)?;
            base_url.join(location)?.to_string()
        };

        Ok(upload_url)
    }

    fn encode_metadata(&self, metadata: &HashMap<String, String>) -> String {
        metadata
            .iter()
            .map(|(k, v)| {
                let encoded_value = STANDARD.encode(v);
                format!("{}={}", k, encoded_value)
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    async fn get_upload_offset(&self, upload_url: &str) -> Result<u64> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Tus-Resumable",
            HeaderValue::from_str(&self.config.tus_version)?,
        );

        if !self.config.headers.is_empty() {
            for (key, value) in &self.config.headers {
                headers.insert(
                    HeaderName::from_bytes(key.as_bytes())?,
                    HeaderValue::from_str(value)?,
                );
            }
        }

        let response = self.client.head(upload_url).headers(headers).send().await?;

        if !response.status().is_success() {
            return Err(TransferError::protocol_specific(
                "TUS",
                format!("Failed to get upload offset: {}", response.status()),
            ));
        }

        let offset = response
            .headers()
            .get("Upload-Offset")
            .ok_or_else(|| TransferError::protocol("No Upload-Offset header in response"))?
            .to_str()
            .map_err(|err| TransferError::Protocol(format!("Invalid Upload-Offset: {}", err)))?
            .parse::<u64>()
            .map_err(|err| TransferError::Protocol(format!("Invalid Upload-Offset: {}", err)))?;

        Ok(offset)
    }

    async fn upload_chunk(&self, upload_url: &str, offset: u64, data: Bytes) -> Result<u64> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Tus-Resumable",
            HeaderValue::from_str(&self.config.tus_version)?,
        );
        headers.insert("Upload-Offset", HeaderValue::from_str(&offset.to_string())?);
        headers.insert(
            "Content-Type",
            HeaderValue::from_static("application/offset+octet-stream"),
        );
        headers.insert(
            "Content-Length",
            HeaderValue::from_str(&data.len().to_string())?,
        );

        if !self.config.headers.is_empty() {
            for (key, value) in &self.config.headers {
                headers.insert(
                    HeaderName::from_bytes(key.as_bytes())?,
                    HeaderValue::from_str(value)?,
                );
            }
        }

        let response = self
            .client
            .patch(upload_url)
            .headers(headers)
            .body(data)
            .send()
            .await?;

        if response.status() != StatusCode::NO_CONTENT {
            return Err(TransferError::protocol_specific(
                "TUS",
                format!("Upload failed: {}", response.status()),
            ));
        }

        let offset = response
            .headers()
            .get("Upload-Offset")
            .ok_or_else(|| TransferError::protocol("No Upload-Offset header in response"))?
            .to_str()
            .map_err(|err| TransferError::Protocol(format!("Invalid Upload-Offset: {}", err)))?
            .parse::<u64>()
            .map_err(|err| TransferError::Protocol(format!("Invalid Upload-Offset: {}", err)))?;

        Ok(offset)
    }
}

#[async_trait]
impl TransferProtocol for TusUploadProtocol {
    fn protocol_type(&self) -> ProtocolType {
        ProtocolType::Tus
    }

    fn transfer_mode(&self) -> TransferMode {
        TransferMode::Upload
    }

    async fn initialize(&self, ctx: &mut TransferContext) -> Result<()> {
        let file_path = Path::new(&ctx.source);
        let file_metadata = tokio::fs::metadata(&file_path).await?;
        let file_size = file_metadata.len();

        let upload_url = if ctx.destination.is_empty() {
            self.create_upload(file_size).await?
        } else {
            ctx.destination.clone()
        };

        let offset = self.get_upload_offset(&upload_url).await?;

        let tus_metadata = TusMetadata {
            upload_url: upload_url.clone(),
            upload_offset: offset,
            upload_length: file_size,
            extensions: vec![],
            upload_expires: None,
            metadata: self.options.metadata.clone(),
        };

        let metadata = TransferMetadata {
            total_size: Some(file_size),
            resumable: true,
            content_type: self.options.metadata.get("filetype").cloned(),
            filename: self.options.metadata.get("filename").cloned(),
            extra: serde_json::to_value(&tus_metadata)?,
        };

        ctx.metadata = Some(metadata);
        ctx.destination = upload_url;
        ctx.transferred_bytes = offset;

        todo!()
    }

    async fn get_progress(&self, ctx: &TransferContext) -> Result<u64> {
        if ctx.destination.is_empty() {
            return Ok(0);
        }

        match self.get_upload_offset(&ctx.destination).await {
            Ok(offset) => Ok(offset),
            Err(_) => Ok(0),
        }
    }

    async fn create_stream(
        &self,
        ctx: &TransferContext,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<TransferChunk>> + Send>>> {
        todo!()
    }

    async fn finalize(&self, ctx: &mut TransferContext) -> Result<()> {
        let expected_size = ctx
            .metadata
            .as_ref()
            .and_then(|m| m.total_size)
            .ok_or_else(|| TransferError::Internal("No file size in metadata".into()))?;

        let actual_offset = self.get_upload_offset(ctx.destination.as_ref()).await?;

        if actual_offset != expected_size {
            return Err(TransferError::Incomplete {
                actual: actual_offset,
                expected: expected_size,
            });
        }

        Ok(())
    }

    async fn cancel(&self, ctx: &TransferContext) -> Result<()> {
        if ctx.destination.is_empty() {
            return Ok(());
        }

        if self.options.delete_on_termination {
            let mut headers = HeaderMap::new();
            headers.insert(
                "Tus-Resumable",
                HeaderValue::from_str(&self.config.tus_version)?,
            );

            for (key, value) in &self.config.headers {
                headers.insert(
                    HeaderName::from_bytes(key.as_bytes())?,
                    HeaderValue::from_str(value)?,
                );
            }

            let response = self
                .client
                .delete(&ctx.destination)
                .headers(headers)
                .send()
                .await?;

            if !response.status().is_success() && response.status() != StatusCode::NOT_FOUND {
                return Err(TransferError::protocol_specific(
                    "TUS",
                    format!("Failed to delete upload: {}", response.status()),
                ));
            }
        }

        Ok(())
    }
}

pub struct TusUploadProcessor {
    protocol: Arc<TusUploadProtocol>,
    upload_url: String,
    current_offset: u64,
}

impl TusUploadProcessor {
    pub fn new(protocol: Arc<TusUploadProtocol>, upload_url: String, init_offset: u64) -> Self {
        Self {
            protocol,
            upload_url,
            current_offset: init_offset,
        }
    }
}

#[async_trait]
impl TransferProcessor for TusUploadProcessor {
    async fn process_chunk(&mut self, chunk: TransferChunk) -> Result<()> {
        todo!()
    }

    async fn finalize(&mut self) -> Result<()> {
        todo!()
    }
}

pub struct TusUploadBuilder {
    source: PathBuf,
    config: TusConfig,
    options: TusUploadOptions,
    transfer_options: TransferOptions,
}

impl TusUploadBuilder {
    pub fn new(source: PathBuf, endpoint: String) -> Self {
        let env_config = get_config();
        let mut config = TusConfig::default();
        config.endpoint = endpoint;
        config
            .headers
            .insert("token".to_string(), env_config.token.clone());

        Self {
            source,
            config,
            options: TusUploadOptions::default(),
            transfer_options: TransferOptions::default(),
        }
    }
}

impl TransferTaskBuilder for TusUploadBuilder {
    fn build_context(&self) -> Result<TransferContext> {
        Ok(TransferContext {
            id: TransferId::new(),
            source: self.source.to_string_lossy().to_string(),
            destination: String::new(),
            mode: TransferMode::Upload,
            protocol: ProtocolType::Tus,
            metadata: None,
            headers: Some(self.config.headers.clone()),
            transferred_bytes: 0,
        })
    }

    fn build_protocol(&self) -> Result<Box<dyn TransferProtocol>> {
        Ok(Box::new(TusUploadProtocol::new(
            self.config.clone(),
            self.options.clone(),
        )?))
    }

    fn build_processor(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn TransferProcessor>>> + Send>> {
        let protocol =
            Arc::new(TusUploadProtocol::new(self.config.clone(), self.options.clone()).unwrap());

        Box::pin(async move {
            Err(TransferError::Internal(
                "TUS processor needs special handling".into(),
            ))
        })
    }
}
