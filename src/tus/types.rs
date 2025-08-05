use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct TusConfig {
    pub endpoint: String,
    pub headers: HashMap<String, String>,
}

impl Default for TusConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            headers: HashMap::new(),
        }
    }
}

/// 控制协议的选项
pub struct TusProtocolOptions {
    /// 元数据信息
    metadata: HashMap<String, String>,
    
    /// 在上传完毕后删除本地资源 
    delete_on_termination: bool,
    
    /// 资源过期时间
    upload_expires_in: Option<u64>,
}
