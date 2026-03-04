use serde::Deserialize;
use tracing::{info, warn};

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    pub api_key: Option<String>,
    pub deepseek_api_key: Option<String>,

    pub base_url: Option<String>,
    pub mad_base_url: Option<String>,

    pub model: Option<String>,
    pub mad_model: Option<String>,

    pub port: Option<u16>,
    pub socket_path: Option<String>,
    pub mad_socket_path: Option<String>,

    pub memory_model: Option<String>,
    pub memory_model_path: Option<String>,
}

impl Config {
    pub fn get_api_key(&self) -> Option<String> {
        self.api_key.clone().or(self.deepseek_api_key.clone())
    }

    pub fn get_base_url(&self) -> Option<String> {
        self.base_url.clone().or(self.mad_base_url.clone())
    }

    pub fn get_model(&self) -> Option<String> {
        self.model.clone().or(self.mad_model.clone())
    }

    pub fn get_memory_model(&self) -> String {
        self.memory_model
            .clone()
            .unwrap_or_else(|| "sentence-transformers/all-MiniLM-L6-v2".to_string())
    }

    pub fn get_memory_model_path(&self) -> Option<String> {
        self.memory_model_path.clone()
    }

    pub fn get_socket_path(&self) -> Option<String> {
        self.socket_path
            .clone()
            .or_else(|| self.mad_socket_path.clone())
    }
}

pub fn default_socket_path() -> String {
    if let Some(home) = dirs::home_dir() {
        return home
            .join(".open-mad")
            .join("mad.sock")
            .to_string_lossy()
            .to_string();
    }
    "/tmp/open-mad.sock".to_string()
}

pub fn init_env() {
    let _ = dotenvy::dotenv();
    if let Some(home) = dirs::home_dir() {
        let _ = dotenvy::from_path(home.join(".open-mad").join(".env"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let _ = dotenvy::from_path(parent.join(".env"));
    }
}

pub fn load_config() -> Config {
    let home = match dirs::home_dir() {
        Some(path) => path,
        None => {
            warn!("Could not find home directory");
            return Config::default();
        }
    };

    let config_path = home.join(".open-mad/config.toml");
    if !config_path.exists() {
        info!(
            "Config file not found at {:?}, using defaults/env",
            config_path
        );
        return Config::default();
    }

    info!("Loading config from {:?}", config_path);
    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read config file: {}", e);
            return Config::default();
        }
    };

    match toml::from_str(&content) {
        Ok(c) => {
            info!("Config loaded successfully from {:?}", config_path);
            c
        }
        Err(e) => {
            warn!("Failed to parse config file: {}", e);
            Config::default()
        }
    }
}
