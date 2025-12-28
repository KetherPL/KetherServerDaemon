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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::NamedTempFile;
    
    // Helper functions to safely modify environment variables in tests
    fn set_env_var(key: &str, value: &str) {
        unsafe {
            std::env::set_var(key, value);
        }
    }
    
    fn remove_env_var(key: &str) {
        unsafe {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.l4d2_server_dir, PathBuf::from("/opt/l4d2/server"));
        assert_eq!(config.registry_db_path, PathBuf::from("registry.db"));
        assert_eq!(config.backend_api_url, "http://localhost:3000/api");
        assert_eq!(config.backend_api_key, None);
        assert_eq!(config.local_api_bind, SocketAddr::from_str("127.0.0.1:8080").unwrap());
        assert_eq!(config.sync_interval_secs, 300);
        assert_eq!(config.log_level, "info");
    }

    #[test]
    fn test_addons_dir() {
        let config = Config::default();
        let addons_dir = config.addons_dir();
        assert_eq!(addons_dir, PathBuf::from("/opt/l4d2/server/left4dead2/addons"));
    }

    #[test]
    fn test_load_missing_config_file() {
        // Save original KETHER_CONFIG env var if it exists
        let original_config = std::env::var("KETHER_CONFIG").ok();
        remove_env_var("KETHER_CONFIG");
        
        // Clear all KETHER_* env vars to test defaults
        remove_env_var("KETHER_L4D2_SERVER_DIR");
        remove_env_var("KETHER_REGISTRY_DB_PATH");
        remove_env_var("KETHER_BACKEND_API_URL");
        remove_env_var("KETHER_BACKEND_API_KEY");
        remove_env_var("KETHER_LOCAL_API_BIND");
        remove_env_var("KETHER_SYNC_INTERVAL_SECS");
        remove_env_var("KETHER_LOG_LEVEL");

        // This should fall back to defaults since config.toml doesn't exist
        let config = Config::load().unwrap();
        assert_eq!(config.l4d2_server_dir, PathBuf::from("/opt/l4d2/server"));
        assert_eq!(config.registry_db_path, PathBuf::from("registry.db"));

        // Restore original env var
        if let Some(val) = original_config {
            set_env_var("KETHER_CONFIG", &val);
        }
    }

    #[test]
    fn test_load_from_toml() {
        let temp_file = NamedTempFile::new().unwrap();
        let config_content = r#"
l4d2_server_dir = "/custom/server/path"
registry_db_path = "/custom/registry.db"
backend_api_url = "http://custom-api.example.com"
backend_api_key = "test-key-123"
local_api_bind = "0.0.0.0:9000"
sync_interval_secs = 600
log_level = "debug"
"#;
        fs::write(temp_file.path(), config_content).unwrap();

        // Save original KETHER_CONFIG env var
        let original_config = std::env::var("KETHER_CONFIG").ok();
        set_env_var("KETHER_CONFIG", temp_file.path().to_str().unwrap());

        // Clear env vars to test pure TOML loading
        remove_env_var("KETHER_L4D2_SERVER_DIR");
        remove_env_var("KETHER_REGISTRY_DB_PATH");
        remove_env_var("KETHER_BACKEND_API_URL");
        remove_env_var("KETHER_BACKEND_API_KEY");
        remove_env_var("KETHER_LOCAL_API_BIND");
        remove_env_var("KETHER_SYNC_INTERVAL_SECS");
        remove_env_var("KETHER_LOG_LEVEL");

        let config = Config::load().unwrap();
        assert_eq!(config.l4d2_server_dir, PathBuf::from("/custom/server/path"));
        assert_eq!(config.registry_db_path, PathBuf::from("/custom/registry.db"));
        assert_eq!(config.backend_api_url, "http://custom-api.example.com");
        assert_eq!(config.backend_api_key, Some("test-key-123".to_string()));
        assert_eq!(config.local_api_bind, SocketAddr::from_str("0.0.0.0:9000").unwrap());
        assert_eq!(config.sync_interval_secs, 600);
        assert_eq!(config.log_level, "debug");

        // Restore original env var
        if let Some(val) = original_config {
            set_env_var("KETHER_CONFIG", &val);
        } else {
            remove_env_var("KETHER_CONFIG");
        }
    }

    #[test]
    fn test_env_var_override_l4d2_server_dir() {
        let original = std::env::var("KETHER_L4D2_SERVER_DIR").ok();
        set_env_var("KETHER_L4D2_SERVER_DIR", "/env/server/path");
        
        remove_env_var("KETHER_CONFIG");
        remove_env_var("KETHER_REGISTRY_DB_PATH");
        remove_env_var("KETHER_BACKEND_API_URL");
        remove_env_var("KETHER_BACKEND_API_KEY");
        remove_env_var("KETHER_LOCAL_API_BIND");
        remove_env_var("KETHER_SYNC_INTERVAL_SECS");
        remove_env_var("KETHER_LOG_LEVEL");

        let config = Config::load().unwrap();
        assert_eq!(config.l4d2_server_dir, PathBuf::from("/env/server/path"));

        if let Some(val) = original {
            set_env_var("KETHER_L4D2_SERVER_DIR", &val);
        } else {
            remove_env_var("KETHER_L4D2_SERVER_DIR");
        }
    }

    #[test]
    fn test_env_var_override_registry_db_path() {
        let original = std::env::var("KETHER_REGISTRY_DB_PATH").ok();
        set_env_var("KETHER_REGISTRY_DB_PATH", "/env/registry.db");
        
        remove_env_var("KETHER_CONFIG");
        remove_env_var("KETHER_L4D2_SERVER_DIR");
        remove_env_var("KETHER_BACKEND_API_URL");
        remove_env_var("KETHER_BACKEND_API_KEY");
        remove_env_var("KETHER_LOCAL_API_BIND");
        remove_env_var("KETHER_SYNC_INTERVAL_SECS");
        remove_env_var("KETHER_LOG_LEVEL");

        let config = Config::load().unwrap();
        assert_eq!(config.registry_db_path, PathBuf::from("/env/registry.db"));

        if let Some(val) = original {
            set_env_var("KETHER_REGISTRY_DB_PATH", &val);
        } else {
            remove_env_var("KETHER_REGISTRY_DB_PATH");
        }
    }

    #[test]
    fn test_env_var_override_backend_api_url() {
        let original = std::env::var("KETHER_BACKEND_API_URL").ok();
        set_env_var("KETHER_BACKEND_API_URL", "http://env-api.example.com");
        
        remove_env_var("KETHER_CONFIG");
        remove_env_var("KETHER_L4D2_SERVER_DIR");
        remove_env_var("KETHER_REGISTRY_DB_PATH");
        remove_env_var("KETHER_BACKEND_API_KEY");
        remove_env_var("KETHER_LOCAL_API_BIND");
        remove_env_var("KETHER_SYNC_INTERVAL_SECS");
        remove_env_var("KETHER_LOG_LEVEL");

        let config = Config::load().unwrap();
        assert_eq!(config.backend_api_url, "http://env-api.example.com");

        if let Some(val) = original {
            set_env_var("KETHER_BACKEND_API_URL", &val);
        } else {
            remove_env_var("KETHER_BACKEND_API_URL");
        }
    }

    #[test]
    fn test_env_var_override_backend_api_key() {
        let original = std::env::var("KETHER_BACKEND_API_KEY").ok();
        set_env_var("KETHER_BACKEND_API_KEY", "env-key-456");
        
        remove_env_var("KETHER_CONFIG");
        remove_env_var("KETHER_L4D2_SERVER_DIR");
        remove_env_var("KETHER_REGISTRY_DB_PATH");
        remove_env_var("KETHER_BACKEND_API_URL");
        remove_env_var("KETHER_LOCAL_API_BIND");
        remove_env_var("KETHER_SYNC_INTERVAL_SECS");
        remove_env_var("KETHER_LOG_LEVEL");

        let config = Config::load().unwrap();
        assert_eq!(config.backend_api_key, Some("env-key-456".to_string()));

        if let Some(val) = original {
            set_env_var("KETHER_BACKEND_API_KEY", &val);
        } else {
            remove_env_var("KETHER_BACKEND_API_KEY");
        }
    }

    #[test]
    fn test_env_var_override_local_api_bind() {
        let original = std::env::var("KETHER_LOCAL_API_BIND").ok();
        set_env_var("KETHER_LOCAL_API_BIND", "192.168.1.1:9090");
        
        remove_env_var("KETHER_CONFIG");
        remove_env_var("KETHER_L4D2_SERVER_DIR");
        remove_env_var("KETHER_REGISTRY_DB_PATH");
        remove_env_var("KETHER_BACKEND_API_URL");
        remove_env_var("KETHER_BACKEND_API_KEY");
        remove_env_var("KETHER_SYNC_INTERVAL_SECS");
        remove_env_var("KETHER_LOG_LEVEL");

        let config = Config::load().unwrap();
        assert_eq!(config.local_api_bind, SocketAddr::from_str("192.168.1.1:9090").unwrap());

        if let Some(val) = original {
            set_env_var("KETHER_LOCAL_API_BIND", &val);
        } else {
            remove_env_var("KETHER_LOCAL_API_BIND");
        }
    }

    #[test]
    fn test_env_var_override_sync_interval_secs() {
        let original = std::env::var("KETHER_SYNC_INTERVAL_SECS").ok();
        set_env_var("KETHER_SYNC_INTERVAL_SECS", "120");
        
        remove_env_var("KETHER_CONFIG");
        remove_env_var("KETHER_L4D2_SERVER_DIR");
        remove_env_var("KETHER_REGISTRY_DB_PATH");
        remove_env_var("KETHER_BACKEND_API_URL");
        remove_env_var("KETHER_BACKEND_API_KEY");
        remove_env_var("KETHER_LOCAL_API_BIND");
        remove_env_var("KETHER_LOG_LEVEL");

        let config = Config::load().unwrap();
        assert_eq!(config.sync_interval_secs, 120);

        if let Some(val) = original {
            set_env_var("KETHER_SYNC_INTERVAL_SECS", &val);
        } else {
            remove_env_var("KETHER_SYNC_INTERVAL_SECS");
        }
    }

    #[test]
    fn test_env_var_override_log_level() {
        let original = std::env::var("KETHER_LOG_LEVEL").ok();
        set_env_var("KETHER_LOG_LEVEL", "trace");
        
        remove_env_var("KETHER_CONFIG");
        remove_env_var("KETHER_L4D2_SERVER_DIR");
        remove_env_var("KETHER_REGISTRY_DB_PATH");
        remove_env_var("KETHER_BACKEND_API_URL");
        remove_env_var("KETHER_BACKEND_API_KEY");
        remove_env_var("KETHER_LOCAL_API_BIND");
        remove_env_var("KETHER_SYNC_INTERVAL_SECS");

        let config = Config::load().unwrap();
        assert_eq!(config.log_level, "trace");

        if let Some(val) = original {
            set_env_var("KETHER_LOG_LEVEL", &val);
        } else {
            remove_env_var("KETHER_LOG_LEVEL");
        }
    }
}

