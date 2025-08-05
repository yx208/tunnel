use std::str::FromStr;
use async_trait::async_trait;
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use url::Url;

use super::types::{TusConfig, TusProtocolOptions};
use crate::core::{
    Result,
    TransferProtocol,
    TransferTaskBuilder,
    TransferContext,
    TransferError
};

pub struct TusProtocol {
    config: TusConfig,
    client: Client,
}

impl TusProtocol {
    pub fn new(config: TusConfig) -> Self {
        Self {
            config,
            client: Client::new()
        }
    }

    pub async fn create_upload(&self, file_size: u64) -> Result<String> {
        let headers = self.create_header_map()?;
        let response = self.client
            .post(&self.config.endpoint)
            .header("Tus-Resumable", "1.0.0")
            .header("Upload-Length", file_size)
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

        let upload_url = self.ensure_location_complete(location)?;

        Ok(upload_url)
    }

    fn create_header_map(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        if !self.config.headers.is_empty() {
            for (k, v) in &self.config.headers {
                headers.insert(
                    HeaderName::from_str(&k)?,
                    HeaderValue::from_str(&v)?
                );
            }
        }

        Ok(headers)
    }

    fn ensure_location_complete(&self, location: &str) -> Result<String> {
        if location.starts_with("http") {
            Ok(location.to_string())
        } else {
            let base_url = Url::parse(&self.config.endpoint)?;
            let final_url = base_url.join(location)?;
            Ok(final_url.to_string())
        }
    }

    pub async fn get_upload_offset(&self, upload_url: &str) -> Result<u64> {
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
    async fn initialize(&self, ctx: &mut TransferContext) -> Result<()> {
        todo!()
    }

    async fn cancel(&self, ctx: &mut TransferContext) -> Result<()> {
        todo!()
    }

    async fn finalize(&self, ctx: &mut TransferContext) -> Result<()> {
        todo!()
    }
}

pub struct TusTaskBuilder {
    config: TusConfig,
    options: Option<TusProtocolOptions>,
    source: String,
}

impl TusTaskBuilder {
    pub fn new(endpoint: String, source: String) -> Self {
        let mut config = TusConfig::default();
        config.endpoint = endpoint;

        Self {
            config,
            source,
            options: None,
        }
    }
}

impl TransferTaskBuilder for TusTaskBuilder {
    fn build_protocol(&self) -> Box<dyn TransferProtocol> {
        Box::new(TusProtocol::new(self.config.clone()))
    }

    fn build_context(&self) -> TransferContext {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use crate::config::get_config;
    use super::*;

    #[tokio::test]
    async fn test_create() {
        let config = get_config();
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), config.token);
        let protocol = TusProtocol::new(TusConfig {
            endpoint: config.endpoint.clone(),
            headers
        });

        let result = protocol.create_upload(1024).await.unwrap();
        println!("{:?}", result);
    }

    #[tokio::test]
    async fn test_offset() {
        let config = get_config();
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), config.token);
        let protocol = TusProtocol::new(TusConfig {
            endpoint: config.endpoint.clone(),
            headers
        });

        let upload_url = protocol.create_upload(1024).await.unwrap();
        let offset = protocol.get_upload_offset(&upload_url).await.unwrap();

        assert_eq!(offset, 0)
    }
}
