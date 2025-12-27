// SPDX-License-Identifier: GPL-3.0-only
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Base Left 4 Dead 2 server directory
    pub l4d2_server_dir: PathBuf,
    
    /// SQLite database path for registry
    pub registry_db_path: PathBuf,
    
    /// Remote backend API endpoint URL
    pub backend_api_url: String,
    
    /// Optional authentication token for backend API
    #[serde(default)]
    pub backend_api_key: Option<String>,
    
    /// Local API bind address (e.g., "127.0.0.1:8080")
    pub local_api_bind: SocketAddr,
    
    /// Backend sync interval in seconds
    pub sync_interval_secs: u64,
    
    /// Logging level (trace, debug, info, warn, error)
    pub log_level: String,
}

impl Config {
    /// Load configuration from TOML file with environment variable overrides
    pub fn load() -> anyhow::Result<Self> {
        let config_path = std::env::var("KETHER_CONFIG")
            .unwrap_or_else(|_| "config.toml".to_string());
        
        let mut config: Config = if std::path::Path::new(&config_path).exists() {
            let contents = std::fs::read_to_string(&config_path)?;
            toml::from_str(&contents)?
        } else {
            // Use default configuration
            Config::default()
        };
        
        // Apply environment variable overrides
        if let Ok(val) = std::env::var("KETHER_L4D2_SERVER_DIR") {
            config.l4d2_server_dir = PathBuf::from(val);
        }
        if let Ok(val) = std::env::var("KETHER_REGISTRY_DB_PATH") {
            config.registry_db_path = PathBuf::from(val);
        }
        if let Ok(val) = std::env::var("KETHER_BACKEND_API_URL") {
            config.backend_api_url = val;
        }
        if let Ok(val) = std::env::var("KETHER_BACKEND_API_KEY") {
            config.backend_api_key = Some(val);
        }
        if let Ok(val) = std::env::var("KETHER_LOCAL_API_BIND") {
            config.local_api_bind = SocketAddr::from_str(&val)?;
        }
        if let Ok(val) = std::env::var("KETHER_SYNC_INTERVAL_SECS") {
            config.sync_interval_secs = val.parse()?;
        }
        if let Ok(val) = std::env::var("KETHER_LOG_LEVEL") {
            config.log_level = val;
        }
        
        Ok(config)
    }
    
    /// Get the addons directory path
    pub fn addons_dir(&self) -> PathBuf {
        self.l4d2_server_dir.join("left4dead2").join("addons")
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            l4d2_server_dir: PathBuf::from("/opt/l4d2/server"),
            registry_db_path: PathBuf::from("registry.db"),
            backend_api_url: String::from("http://localhost:3000/api"),
            backend_api_key: None,
            local_api_bind: SocketAddr::from_str("127.0.0.1:8080").unwrap(),
            sync_interval_secs: 300, // 5 minutes
            log_level: String::from("info"),
        }
    }
}

