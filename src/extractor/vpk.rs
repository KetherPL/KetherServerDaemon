// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::path::PathBuf;
use tracing::info;
use regex::Regex;
use sourcepak::{
    common::file::VPKFileReader,
    common::format::PakReader,
    pak::v1::format::VPKVersion1,
};
use crate::extractor::traits::{Extractor, VpkMetadata};

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
}

#[async_trait]
impl Extractor for VpkExtractor {
    async fn extract_zip(&self, _archive_path: PathBuf, _dest: PathBuf) -> anyhow::Result<()> {
        Err(anyhow::anyhow!("ZIP extraction not supported by VpkExtractor"))
    }
    
    async fn extract_vpk(&self, _archive_path: PathBuf, _dest: PathBuf) -> anyhow::Result<()> {
        // VPK files don't need extraction - they are used as-is by the game engine
        // Metadata is extracted separately via extract_vpk_metadata
        info!("VPK files do not require extraction - they are used directly by the game");
        Ok(())
    }
    
    async fn extract_vpk_metadata(&self, archive_path: PathBuf) -> anyhow::Result<VpkMetadata> {
        let archive_path_clone = archive_path.clone();
        
        tokio::task::spawn_blocking(move || {
            let path = Path::new(&archive_path_clone);
            let mut file = File::open(path)?;
            
            // Read VPK version 1 format
            let vpk = VPKVersion1::try_from(&mut file)
                .map_err(|e| anyhow::anyhow!("Failed to read VPK file: {}", e))?;
            
            // The key for "addoninfo.txt" at the root of the VPK is " /addoninfo.txt"
            // because the root path is a space, and sourcepak builds keys as "{path}/{file_name}.{extension}"
            let addoninfo_key = " /addoninfo.txt";
            let entry = vpk.tree.files.get(addoninfo_key)
                .ok_or_else(|| anyhow::anyhow!("addoninfo.txt not found in VPK"))?;
            
            let archive_dir = path.parent()
                .unwrap_or_else(|| Path::new("."))
                .to_string_lossy();
            let vpk_name = path.file_stem()
                .unwrap_or_default()
                .to_string_lossy();
            let base_vpk_name = vpk_name.strip_suffix("_dir").unwrap_or(&vpk_name);
            
            // Read the addoninfo.txt content
            // Handle both embedded files (archive_index == 0x7FFF) and split archives
            let content_bytes = if entry.archive_index == 0x7FFF {
                // For VPK v1, the tree starts immediately after the header.
                // The data block for embedded files starts immediately after the tree.
                let tree_offset = std::mem::size_of_val(&vpk.header) as u64;
                let seek_pos = tree_offset + vpk.header.tree_size as u64 + entry.entry_offset as u64;
                file.seek(SeekFrom::Start(seek_pos))?;
                file.read_bytes(entry.entry_length as usize)
                    .map_err(|e| anyhow::anyhow!("Failed to read embedded file data from VPK: {}", e))?
            } else {
                // Fallback to sourcepak for other archive types (e.g., pak01_001.vpk)
                vpk.read_file(&archive_dir.to_string(), &base_vpk_name.to_string(), &addoninfo_key.to_string())
                    .ok_or_else(|| anyhow::anyhow!("Failed to read addoninfo.txt from split archive"))?
            };
            
            let content = String::from_utf8_lossy(&content_bytes).into_owned();
            
            // Parse metadata from KeyValue format
            let mut title: Option<String> = None;
            let mut version: Option<String> = None;
            
            for line in content.lines() {
                if title.is_none() {
                    if let Some(val) = Self::extract_value(line, "addonTitle") {
                        title = Some(val);
                    }
                }
                if version.is_none() {
                    if let Some(val) = Self::extract_value(line, "addonVersion") {
                        version = Some(val);
                    }
                }
            }
            
            Ok(VpkMetadata {
                title: title.unwrap_or_else(|| "Unknown".to_string()),
                version: version.unwrap_or_else(|| "Unknown".to_string()),
            })
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
    fn test_extract_value_multiple_quotes() {
        let line = r#"  "addonTitle" "Test \"Quoted\" Map""#;
        let result = VpkExtractor::extract_value(line, "addonTitle");
        // Should extract up to the closing quote
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn test_extract_zip_not_supported() {
        let extractor = VpkExtractor::new();
        let temp_dir = TempDir::new().unwrap();
        let fake_zip = temp_dir.path().join("test.zip");
        std::fs::write(&fake_zip, b"fake zip").unwrap();
        
        let dest_dir = TempDir::new().unwrap();
        let dest_path = dest_dir.path().to_path_buf();
        
        let result = extractor.extract_zip(fake_zip, dest_path).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("ZIP extraction not supported"));
    }

    #[tokio::test]
    async fn test_extract_vpk_succeeds() {
        let extractor = VpkExtractor::new();
        let temp_dir = TempDir::new().unwrap();
        let fake_vpk = temp_dir.path().join("test.vpk");
        std::fs::write(&fake_vpk, b"fake vpk").unwrap();
        
        let dest_dir = TempDir::new().unwrap();
        let dest_path = dest_dir.path().to_path_buf();
        
        // VPK extraction should succeed (just logs that extraction is not needed)
        let result = extractor.extract_vpk(fake_vpk, dest_path).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_extract_vpk_metadata_nonexistent_file() {
        let extractor = VpkExtractor::new();
        let nonexistent_vpk = PathBuf::from("/nonexistent/path/test.vpk");
        
        let result = extractor.extract_vpk_metadata(nonexistent_vpk).await;
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
        // Should error because it's not a valid VPK format
    }

    // Note: Testing with real VPK files would require either:
    // 1. Creating minimal valid VPK files as test fixtures
    // 2. Using mock objects for the VPK reader
    // 3. Using actual VPK files from the game
    // For now, we test the helper functions and error paths that don't require valid VPK files.
}

