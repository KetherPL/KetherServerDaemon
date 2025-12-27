// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use std::path::PathBuf;

/// Metadata extracted from a VPK file
#[derive(Debug, Clone)]
pub struct VpkMetadata {
    pub title: String,
    pub version: String,
}

#[async_trait]
pub trait Extractor: Send + Sync {
    /// Extract a ZIP archive to the destination directory
    async fn extract_zip(&self, archive_path: PathBuf, dest: PathBuf) -> anyhow::Result<()>;
    
    /// Extract a VPK archive to the destination directory
    /// Note: VPK files don't need extraction, use extract_vpk_metadata to get metadata
    async fn extract_vpk(&self, archive_path: PathBuf, dest: PathBuf) -> anyhow::Result<()>;
    
    /// Extract metadata (Title and Version) from a VPK file
    async fn extract_vpk_metadata(&self, archive_path: PathBuf) -> anyhow::Result<VpkMetadata>;
}

