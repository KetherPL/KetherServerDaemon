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
        backend_api_url: Config::default().backend_api_url,
        backend_api_key: None,
        local_api_bind: SocketAddr::from_str("127.0.0.1:0").unwrap(),
        sync_interval_secs: 60,
        log_level: "error".to_string(),
        max_download_size_bytes: 100 * 1024 * 1024,
        max_extraction_size_bytes: 1024 * 1024 * 1024,
        max_extraction_file_count: 10000,
        l4d2center_index_url: Config::default().l4d2center_index_url,
        hidden_workshop_ids: Vec::new(),
        hidden_map_ids: Vec::new(),
        map_update_check_interval_days: Config::default().map_update_check_interval_days,
        workshop_update_check_enabled: Config::default().workshop_update_check_enabled,
        workshop_update_auto_apply: Config::default().workshop_update_auto_apply,
        l4d2center_update_check_enabled: Config::default().l4d2center_update_check_enabled,
        l4d2center_update_auto_apply: Config::default().l4d2center_update_auto_apply,
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

/// Write a minimal valid VPK v1 with embedded `addoninfo.txt` for install/discovery tests.
#[cfg(test)]
pub fn write_minimal_test_vpk(path: &Path, title: &str) -> anyhow::Result<()> {
    use crc::{Crc, CRC_32_ISO_HDLC};
    use sourcepak::common::format::PakReader;
    use sourcepak::common::file::VPKFileWriter;
    use sourcepak::common::format::{PakWriter, VPKDirectoryEntry, VPKTree};
    use sourcepak::pak::v1::format::{
        VPKHeaderV1, VPKVersion1, VPK_SIGNATURE_V1, VPK_VERSION_V1,
    };
    use std::fs::File;

    let content = format!("\"addonTitle\" \"{title}\"\n\"addonVersion\" \"1.0\"\n");
    let content_bytes = content.as_bytes();
    let crc_val = Crc::<u32>::new(&CRC_32_ISO_HDLC).checksum(content_bytes);

    let file_key = " /addoninfo.txt".to_string();
    let entry = VPKDirectoryEntry {
        crc: crc_val,
        preload_length: content_bytes.len() as u16,
        archive_index: 0,
        entry_offset: 0,
        entry_length: 0,
        terminator: 0xFFFF,
    };

    let mut tree = VPKTree::new();
    tree.files.insert(file_key.clone(), entry);
    tree.preload.insert(file_key, content_bytes.to_vec());

    let tree_temp = tempfile::NamedTempFile::new()?;
    {
        let mut tree_file = File::create(tree_temp.path())?;
        tree.write(&mut tree_file)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    let tree_size = std::fs::metadata(tree_temp.path())?.len() as u32;

    let mut vpk = VPKVersion1::new();
    vpk.header = VPKHeaderV1 {
        signature: VPK_SIGNATURE_V1,
        version: VPK_VERSION_V1,
        tree_size,
    };
    vpk.tree = tree;

    let output_path = path.to_string_lossy().into_owned();
    vpk.write_dir(&output_path)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}
