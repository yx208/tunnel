//! Dev only

use std::fs;
use std::sync::{Mutex, OnceLock};
// use std::cell::OnceCell;
use serde::Deserialize;

static CONFIG: OnceLock<Mutex<Config>> = OnceLock::new();

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    pub endpoint: String,
    pub file_path: String,
    pub file_name: String,
    pub token: String,
}

impl Config {
    pub fn load_config() -> Config {
        let config_str = fs::read_to_string("config.toml")
            .expect("Something went wrong reading the file");
        toml::from_str(&config_str).expect("Can't load config.toml")
    }
}

pub fn init_config() {
    CONFIG.get_or_init(|| {
        let config = Config::load_config();
        Mutex::new(config)
    });
}

pub fn get_config() -> Config {
    init_config();
    CONFIG.get().unwrap().lock().unwrap().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_something() {
        let config = Config::load_config();
        assert!(config.endpoint.starts_with("http"));
    }
}

