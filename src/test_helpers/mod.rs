// SPDX-License-Identifier: GPL-3.0-only
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::Config;
use crate::registry::{JsonRegistry, traits::Registry};
use tempfile::TempDir;

/// JSON registry backed by a temporary directory that is removed on drop.
pub struct TestDatabase {
    pub registry: JsonRegistry,
    _dir: TempDir,
}

/// Holds all temporary directories for an integration-style test setup.
pub struct TestDirs {
    _registry_dir: TempDir,
    addons_dir: TempDir,
    _download_dir: TempDir,
}

impl TestDirs {
    pub fn addons_path(&self) -> &Path {
        self.addons_dir.path()
    }
}

/// Create a JSON registry database inside a temporary directory.
pub async fn setup_test_database() -> anyhow::Result<TestDatabase> {
    let dir = TempDir::new()?;
    let registry_path = dir.path().join("registry.json");
    let registry = JsonRegistry::new(&registry_path).await?;
    Ok(TestDatabase {
        registry,
        _dir: dir,
    })
}

/// Create a test configuration with paths inside a temporary directory.
pub fn create_test_config() -> (Config, TempDir) {
    use std::net::SocketAddr;
    use std::str::FromStr;

    let dir = TempDir::new().expect("Failed to create temp directory");
    let base = dir.path().to_path_buf();

    let config = Config {
        l4d2_server_dir: base.clone(),
        registry_path: base.join("registry.json"),
        backend_api_url: "http://localhost:3000/api".to_string(),
        backend_api_key: None,
        local_api_bind: SocketAddr::from_str("127.0.0.1:0").unwrap(),
        sync_interval_secs: 60,
        log_level: "error".to_string(),
        max_download_size_bytes: 100 * 1024 * 1024,
        max_extraction_size_bytes: 1024 * 1024 * 1024,
        max_extraction_file_count: 10000,
    };

    (config, dir)
}

/// Create a temporary directory for tests.
pub fn create_temp_dir() -> TempDir {
    TempDir::new().expect("Failed to create temp directory")
}

/// Build shared test directories and a registry wrapped for service use.
pub async fn setup_test_dirs() -> anyhow::Result<(Arc<dyn Registry>, TestDirs)> {
    let db = setup_test_database().await?;
    let registry: Arc<dyn Registry> = Arc::new(db.registry);
    let addons_dir = create_temp_dir();
    let download_dir = create_temp_dir();
    let dirs = TestDirs {
        _registry_dir: db._dir,
        addons_dir,
        _download_dir: download_dir,
    };
    Ok((registry, dirs))
}

/// Paths used when constructing `MapInstallationService` in tests.
pub struct TestServicePaths {
    pub addons_dir: PathBuf,
    pub download_dir: PathBuf,
}

impl TestDirs {
    pub fn service_paths(&self) -> TestServicePaths {
        TestServicePaths {
            addons_dir: self.addons_dir.path().to_path_buf(),
            download_dir: self._download_dir.path().to_path_buf(),
        }
    }
}
