use std::collections::HashMap;

pub struct TusConfig {
    pub endpoint: String,
    pub headers: HashMap<String, String>,
}
