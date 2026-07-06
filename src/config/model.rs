// SPDX-License-Identifier: GPL-3.0-only
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Base Left 4 Dead 2 server directory
    pub l4d2_server_dir: PathBuf,

    /// JSON registry file path
    pub registry_path: PathBuf,

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

    /// Maximum download size in bytes (default: 1GB)
    #[serde(default = "default_max_download_size")]
    pub max_download_size_bytes: u64,

    /// Maximum extraction size in bytes (default: 1GB)
    #[serde(default = "default_max_extraction_size")]
    pub max_extraction_size_bytes: u64,

    /// Maximum number of files in an archive (default: 10000)
    #[serde(default = "default_max_extraction_file_count")]
    pub max_extraction_file_count: u64,

    /// L4D2Center server map catalog index URL
    #[serde(default = "default_l4d2center_index_url")]
    pub l4d2center_index_url: String,

    /// Steam Workshop addon IDs hidden from website API and backend sync
    #[serde(default)]
    pub hidden_workshop_ids: Vec<u64>,

    /// Daemon registry map IDs hidden from website API and backend sync
    #[serde(default)]
    pub hidden_map_ids: Vec<u64>,
}

fn default_max_download_size() -> u64 {
    1024 * 1024 * 1024 // 1GB — L4D2 workshop campaigns often exceed 100MB
}

fn default_max_extraction_size() -> u64 {
    1024 * 1024 * 1024 // 1GB
}

fn default_max_extraction_file_count() -> u64 {
    10000
}

fn default_l4d2center_index_url() -> String {
    "https://l4d2center.com/maps/servers/index.json".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            l4d2_server_dir: PathBuf::from("/home/steam/l4d2"),
            registry_path: PathBuf::from("registry.json"),
            backend_api_url: String::from("http://127.0.0.1:3001/api"),
            backend_api_key: None,
            local_api_bind: SocketAddr::from_str("127.0.0.1:8080").unwrap(),
            sync_interval_secs: 300, // 5 minutes
            log_level: String::from("info"),
            max_download_size_bytes: default_max_download_size(),
            max_extraction_size_bytes: default_max_extraction_size(),
            max_extraction_file_count: default_max_extraction_file_count(),
            l4d2center_index_url: default_l4d2center_index_url(),
            hidden_workshop_ids: Vec::new(),
            hidden_map_ids: Vec::new(),
        }
    }
}

impl Config {
    /// Generate a commented default config.toml for first-run setup.
    pub fn generate_toml_with_comments() -> String {
        let defaults = Config::default();
        format!(
            r#"# KetherServerDaemon configuration
# Environment variables (KETHER_*) override these values.
# Saving this file reloads live fields without restart (denylist, sync interval, backend URL, etc.).
# Restart is required for paths, bind address, download limits, and log level.

# Base Left 4 Dead 2 server directory (addons at {{dir}}/left4dead2/addons)
l4d2_server_dir = "{}"

# JSON map registry file path
registry_path = "{}"

# Website-server registry sync API (port 3001)
backend_api_url = "{}"

# Bearer token for backend sync (must match website-server [server_daemon].sync_api_key)
# backend_api_key = "your-shared-secret"

# Local HTTP API bind address
local_api_bind = "{}"

# Backend sync interval in seconds
sync_interval_secs = {}

# Logging level: trace, debug, info, warn, error
log_level = "{}"

# Maximum download size in bytes (default 1 GiB)
max_download_size_bytes = {}

# Maximum ZIP extraction size in bytes (default 1 GiB)
max_extraction_size_bytes = {}

# Maximum number of files extracted from a single archive
max_extraction_file_count = {}

# L4D2Center server map catalog index URL
l4d2center_index_url = "{}"

# Maps hidden from website (still installed on server, visible in REPL)
hidden_workshop_ids = []
hidden_map_ids = []
"#,
            defaults.l4d2_server_dir.display(),
            defaults.registry_path.display(),
            defaults.backend_api_url,
            defaults.local_api_bind,
            defaults.sync_interval_secs,
            defaults.log_level,
            defaults.max_download_size_bytes,
            defaults.max_extraction_size_bytes,
            defaults.max_extraction_file_count,
            defaults.l4d2center_index_url,
        )
    }
}
