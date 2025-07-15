use super::types::{TusConfig, TusUploadOptions};
use crate::config::get_config;
use crate::core::{
    ProtocolType, Result, TransferChunk, TransferContext, TransferError, TransferId, TransferMode,
    TransferOptions, TransferProcessor, TransferProtocol, TransferTaskBuilder,
};
use async_trait::async_trait;
use futures::Stream;
use reqwest::Client;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

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
}

#[async_trait]
impl TransferProtocol for TusUploadProtocol {
    fn protocol_type(&self) -> ProtocolType {
        todo!()
    }

    fn transfer_mode(&self) -> TransferMode {
        todo!()
    }

    async fn initialize(&self, ctx: &mut TransferContext) -> Result<()> {
        todo!()
    }

    async fn get_progress(&self, ctx: &TransferContext) -> Result<u64> {
        todo!()
    }

    async fn create_stream(
        &self,
        ctx: &TransferContext,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<TransferChunk>> + Send>>> {
        todo!()
    }

    async fn finalize(&self, ctx: &mut TransferContext) -> Result<()> {
        todo!()
    }

    async fn cancel(&self, ctx: &TransferContext) -> Result<()> {
        todo!()
    }

    async fn verify_resumable(&self, ctx: &TransferContext) -> Result<bool> {
        todo!()
    }
}

#[async_trait]
impl TransferProcessor for TusUploadProtocol {
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
