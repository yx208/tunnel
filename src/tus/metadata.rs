use std::collections::HashMap;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub filename: Option<String>,
    pub filetype: Option<String>,
    pub custom: HashMap<String, String>,
}

impl Metadata {
    pub fn new() -> Self {
        Self {
            filename: None,
            filetype: None,
            custom: HashMap::new(),
        }
    }

    pub fn to_header(&self) -> String {
        let mut parts = Vec::new();

        if let Some(filename) = &self.filename {
            parts.push(format!("filename {}", STANDARD.encode(filename)));
        }

        if let Some(filetype) = &self.filetype {
            parts.push(format!("filetype {}", STANDARD.encode(filetype)));
        }

        for (key, value) in &self.custom {
            parts.push(format!("{} {}", key, STANDARD.encode(value)));
        }

        parts.join(",")
    }
}


