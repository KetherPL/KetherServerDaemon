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

