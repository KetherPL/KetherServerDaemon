// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use std::path::PathBuf;

#[async_trait]
pub trait Downloader: Send + Sync {
    /// Download a map from Steam Workshop
    async fn download_workshop(&self, workshop_id: u64) -> anyhow::Result<PathBuf>;
    
    /// Download a ZIP file from a URL
    async fn download_zip(&self, url: &str) -> anyhow::Result<PathBuf>;
}

