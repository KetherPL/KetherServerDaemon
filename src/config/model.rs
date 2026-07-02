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

impl Default for Config {
    fn default() -> Self {
        Self {
            l4d2_server_dir: PathBuf::from("/home/steam/l4d2"),
            registry_path: PathBuf::from("registry.json"),
            backend_api_url: String::from("http://localhost:3000/api"),
            backend_api_key: None,
            local_api_bind: SocketAddr::from_str("127.0.0.1:8080").unwrap(),
            sync_interval_secs: 300, // 5 minutes
            log_level: String::from("info"),
            max_download_size_bytes: default_max_download_size(),
            max_extraction_size_bytes: default_max_extraction_size(),
            max_extraction_file_count: default_max_extraction_file_count(),
        }
    }
}
