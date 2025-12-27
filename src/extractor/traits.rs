// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use std::path::PathBuf;

#[async_trait]
pub trait Extractor: Send + Sync {
    /// Extract a ZIP archive to the destination directory
    async fn extract_zip(&self, archive_path: PathBuf, dest: PathBuf) -> anyhow::Result<()>;
    
    /// Extract a VPK archive to the destination directory
    async fn extract_vpk(&self, archive_path: PathBuf, dest: PathBuf) -> anyhow::Result<()>;
}

