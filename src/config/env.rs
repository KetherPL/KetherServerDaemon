// SPDX-License-Identifier: GPL-3.0-only
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

use crate::config::model::Config;

pub mod keys {
    pub const CONFIG: &str = "KETHER_CONFIG";
    pub const L4D2_SERVER_DIR: &str = "KETHER_L4D2_SERVER_DIR";
    pub const REGISTRY_PATH: &str = "KETHER_REGISTRY_PATH";
    pub const BACKEND_API_URL: &str = "KETHER_BACKEND_API_URL";
    pub const BACKEND_API_KEY: &str = "KETHER_BACKEND_API_KEY";
    pub const LOCAL_API_BIND: &str = "KETHER_LOCAL_API_BIND";
    pub const SYNC_INTERVAL_SECS: &str = "KETHER_SYNC_INTERVAL_SECS";
    pub const LOG_LEVEL: &str = "KETHER_LOG_LEVEL";
    pub const MAX_DOWNLOAD_SIZE_BYTES: &str = "KETHER_MAX_DOWNLOAD_SIZE_BYTES";
    pub const MAX_EXTRACTION_SIZE_BYTES: &str = "KETHER_MAX_EXTRACTION_SIZE_BYTES";
    pub const MAX_EXTRACTION_FILE_COUNT: &str = "KETHER_MAX_EXTRACTION_FILE_COUNT";
}

/// Apply environment variable overrides on top of file/default configuration.
pub fn apply_env_overrides(config: &mut Config) -> anyhow::Result<()> {
    if let Ok(val) = std::env::var(keys::L4D2_SERVER_DIR) {
        config.l4d2_server_dir = PathBuf::from(val);
    }
    if let Ok(val) = std::env::var(keys::REGISTRY_PATH) {
        config.registry_path = PathBuf::from(val);
    }
    if let Ok(val) = std::env::var(keys::BACKEND_API_URL) {
        config.backend_api_url = val;
    }
    if let Ok(val) = std::env::var(keys::BACKEND_API_KEY) {
        config.backend_api_key = Some(val);
    }
    if let Ok(val) = std::env::var(keys::LOCAL_API_BIND) {
        config.local_api_bind = SocketAddr::from_str(&val)?;
    }
    if let Ok(val) = std::env::var(keys::SYNC_INTERVAL_SECS) {
        config.sync_interval_secs = val.parse()?;
    }
    if let Ok(val) = std::env::var(keys::LOG_LEVEL) {
        config.log_level = val;
    }
    if let Ok(val) = std::env::var(keys::MAX_DOWNLOAD_SIZE_BYTES) {
        config.max_download_size_bytes = val.parse()?;
    }
    if let Ok(val) = std::env::var(keys::MAX_EXTRACTION_SIZE_BYTES) {
        config.max_extraction_size_bytes = val.parse()?;
    }
    if let Ok(val) = std::env::var(keys::MAX_EXTRACTION_FILE_COUNT) {
        config.max_extraction_file_count = val.parse()?;
    }

    Ok(())
}
