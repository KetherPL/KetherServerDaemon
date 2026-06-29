// SPDX-License-Identifier: GPL-3.0-only
use std::path::PathBuf;
use crate::registry::JsonRegistry;
use crate::config::Config;

/// Create a JSON registry database for testing
pub async fn setup_test_database() -> anyhow::Result<JsonRegistry> {
    let registry_path = PathBuf::from(std::env::temp_dir()).join(format!(
        "kether-test-{}.json",
        uuid::Uuid::new_v4()
    ));
    JsonRegistry::new(&registry_path).await
}

/// Create a test configuration with temporary paths
pub fn create_test_config() -> Config {
    use std::net::SocketAddr;
    use std::str::FromStr;
    
    let temp_dir = std::env::temp_dir().join(format!("kether-test-{}", uuid::Uuid::new_v4()));
    
    Config {
        l4d2_server_dir: temp_dir.clone(),
        registry_path: temp_dir.join("registry.json"),
        backend_api_url: "http://localhost:3000/api".to_string(),
        backend_api_key: None,
        local_api_bind: SocketAddr::from_str("127.0.0.1:0").unwrap(), // Use port 0 to auto-assign
        sync_interval_secs: 60,
        log_level: "error".to_string(), // Reduce log noise in tests
        max_download_size_bytes: 100 * 1024 * 1024, // 100MB
        max_extraction_size_bytes: 1024 * 1024 * 1024, // 1GB
        max_extraction_file_count: 10000,
    }
}

/// Create a temporary directory for tests
pub fn create_temp_dir() -> tempfile::TempDir {
    tempfile::TempDir::new().expect("Failed to create temp directory")
}

