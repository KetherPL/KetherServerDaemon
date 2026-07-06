// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use std::fs::File;
use std::io::Seek;
use std::path::{Path, PathBuf};
use tracing::info;
use regex::Regex;

use crate::extractor::traits::{Extractor, VpkMetadata};
use crate::extractor::vpk_v1::{self, VpkV1Header};

pub struct VpkExtractor;

impl VpkExtractor {
    pub fn new() -> Self {
        Self
    }

    /// Helper function to extract the string value from a KeyValue formatted line
    fn extract_value(line: &str, key: &str) -> Option<String> {
        let pattern = format!(r#"^\s*(?i)(?:"{0}"|{0})\s+"([^"]*)""#, regex::escape(key));
        let re = Regex::new(&pattern).unwrap();
        re.captures(line)
            .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
    }

    fn parse_workshop_id(url: &str) -> Option<u64> {
        let re = Regex::new(r"(?i)steamcommunity\.com/.*[?&]id=(\d+)").ok()?;
        re.captures(url)?
            .get(1)?
            .as_str()
            .parse::<u64>()
            .ok()
    }

    fn parse_metadata_from_bytes(content_bytes: &[u8]) -> VpkMetadata {
        let content = String::from_utf8_lossy(content_bytes);

        let mut title: Option<String> = None;
        let mut version: Option<String> = None;
        let mut addon_url: Option<String> = None;

        for line in content.lines() {
            if title.is_none()
                && let Some(val) = Self::extract_value(line, "addonTitle")
            {
                title = Some(val);
            }
            if version.is_none()
                && let Some(val) = Self::extract_value(line, "addonVersion")
            {
                version = Some(val);
            }
            if addon_url.is_none()
                && let Some(val) = Self::extract_value(line, "addonURL0")
            {
                addon_url = Some(val);
            }
        }

        let workshop_id = addon_url
            .as_deref()
            .and_then(Self::parse_workshop_id);

        VpkMetadata {
            title: title.unwrap_or_else(|| "Unknown".to_string()),
            version: version.unwrap_or_else(|| "Unknown".to_string()),
            workshop_id,
        }
    }

    fn extract_vpk_metadata_blocking(archive_path: PathBuf) -> anyhow::Result<VpkMetadata> {
        let path = Path::new(&archive_path);
        let mut file = File::open(path)?;

        let header = vpk_v1::read_header(&mut file)?;

        if let Some(entry) = vpk_v1::find_addoninfo_entry(&mut file, &header)? {
            let content_bytes =
                vpk_v1::read_addoninfo_bytes(&mut file, path, &header, &entry)?;
            return Ok(Self::parse_metadata_from_bytes(&content_bytes));
        }

        Self::extract_vpk_metadata_sourcepak_fallback(path, &header)
    }

    fn extract_vpk_metadata_sourcepak_fallback(
        path: &Path,
        header: &VpkV1Header,
    ) -> anyhow::Result<VpkMetadata> {
        use sourcepak::{
            common::file::VPKFileReader,
            common::format::PakReader,
            pak::v1::format::VPKVersion1,
        };

        let mut file = File::open(path)?;
        let vpk = VPKVersion1::try_from(&mut file)
            .map_err(|e| anyhow::anyhow!("Failed to read VPK file: {}", e))?;

        let addoninfo_key = " /addoninfo.txt";
        let entry = vpk
            .tree
            .files
            .get(addoninfo_key)
            .ok_or_else(|| anyhow::anyhow!("addoninfo.txt not found in VPK"))?;

        let archive_dir = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_string_lossy();
        let vpk_name = path.file_stem().unwrap_or_default().to_string_lossy();
        let base_vpk_name = vpk_name.strip_suffix("_dir").unwrap_or(&vpk_name);

        let content_bytes = if entry.archive_index == vpk_v1::VPK_EMBEDDED_ARCHIVE_INDEX {
            let tree_offset = std::mem::size_of_val(&vpk.header) as u64;
            let seek_pos = tree_offset + header.tree_size as u64 + entry.entry_offset as u64;
            file.seek(std::io::SeekFrom::Start(seek_pos))?;
            file.read_bytes(entry.entry_length as usize)
                .map_err(|e| anyhow::anyhow!("Failed to read embedded file data from VPK: {}", e))?
        } else {
            vpk.read_file(
                &archive_dir.to_string(),
                &base_vpk_name.to_string(),
                &addoninfo_key.to_string(),
            )
            .ok_or_else(|| anyhow::anyhow!("Failed to read addoninfo.txt from split archive"))?
        };

        Ok(Self::parse_metadata_from_bytes(&content_bytes))
    }
}

#[async_trait]
impl Extractor for VpkExtractor {
    async fn extract_zip(&self, _archive_path: PathBuf, _dest: PathBuf) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "ZIP extraction not supported by VpkExtractor"
        ))
    }

    async fn extract_vpk(&self, _archive_path: PathBuf, _dest: PathBuf) -> anyhow::Result<()> {
        info!("VPK files do not require extraction - they are used directly by the game");
        Ok(())
    }

    async fn extract_sevenz(&self, _archive_path: PathBuf, _dest: PathBuf) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "7z extraction not supported by VpkExtractor"
        ))
    }

    async fn extract_vpk_metadata(&self, archive_path: PathBuf) -> anyhow::Result<VpkMetadata> {
        tokio::task::spawn_blocking(move || {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Self::extract_vpk_metadata_blocking(archive_path)
            })) {
                Ok(result) => result,
                Err(_) => Err(anyhow::anyhow!(
                    "VPK metadata extraction panicked (sourcepak cannot decode non-UTF-8 paths in this VPK)"
                )),
            }
        })
        .await?
    }
}

impl Default for VpkExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers;
    use tempfile::TempDir;

    #[test]
    fn test_extract_value_valid() {
        let line = r#"  "addonTitle" "Test Map""#;
        let result = VpkExtractor::extract_value(line, "addonTitle");
        assert_eq!(result, Some("Test Map".to_string()));
    }

    #[test]
    fn test_extract_value_case_insensitive() {
        let line = r#"  "ADDONTITLE" "Test Map Uppercase""#;
        let result = VpkExtractor::extract_value(line, "addonTitle");
        assert_eq!(result, Some("Test Map Uppercase".to_string()));
    }

    #[test]
    fn test_extract_value_without_quotes_around_key() {
        let line = r#"  addonTitle "Test Map No Quotes""#;
        let result = VpkExtractor::extract_value(line, "addonTitle");
        assert_eq!(result, Some("Test Map No Quotes".to_string()));
    }

    #[test]
    fn test_extract_value_version() {
        let line = r#"    "addonVersion" "1.2.3""#;
        let result = VpkExtractor::extract_value(line, "addonVersion");
        assert_eq!(result, Some("1.2.3".to_string()));
    }

    #[test]
    fn test_extract_value_not_found() {
        let line = r#"  "otherKey" "some value""#;
        let result = VpkExtractor::extract_value(line, "addonTitle");
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_value_empty_line() {
        let line = "";
        let result = VpkExtractor::extract_value(line, "addonTitle");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_workshop_id_filedetails_url() {
        let url = "http://steamcommunity.com/workshop/filedetails/?id=121786282";
        let workshop_id = VpkExtractor::parse_workshop_id(url);
        assert_eq!(workshop_id, Some(121786282));
    }

    #[tokio::test]
    async fn test_extract_vpk_metadata_from_minimal_fixture() {
        let temp_dir = TempDir::new().unwrap();
        let vpk_path = temp_dir.path().join("test_map.vpk");
        test_helpers::write_minimal_test_vpk(&vpk_path, "Fixture Map").unwrap();

        let extractor = VpkExtractor::new();
        let metadata = extractor
            .extract_vpk_metadata(vpk_path)
            .await
            .unwrap();
        assert_eq!(metadata.title, "Fixture Map");
        assert_eq!(metadata.version, "1.0");
    }

    #[tokio::test]
    async fn test_extract_vpk_metadata_nonexistent_file() {
        let extractor = VpkExtractor::new();
        let result = extractor
            .extract_vpk_metadata(PathBuf::from("/nonexistent/path/test.vpk"))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_extract_vpk_metadata_invalid_file() {
        let extractor = VpkExtractor::new();
        let temp_dir = TempDir::new().unwrap();
        let fake_vpk = temp_dir.path().join("test.vpk");
        std::fs::write(&fake_vpk, b"This is not a valid VPK file").unwrap();

        let result = extractor.extract_vpk_metadata(fake_vpk).await;
        assert!(result.is_err());
    }
}
