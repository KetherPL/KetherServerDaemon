// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use std::path::PathBuf;
use tracing::{info, warn};
use crate::downloader::{client::HttpClient, traits::Downloader};

pub struct WorkshopDownloader {
    client: HttpClient,
    temp_dir: PathBuf,
}

impl WorkshopDownloader {
    pub fn new(temp_dir: PathBuf) -> anyhow::Result<Self> {
        Ok(Self {
            client: HttpClient::new()?,
            temp_dir,
        })
    }
    
    async fn download_workshop_item(&self, workshop_id: u64) -> anyhow::Result<PathBuf> {
        // Steam Workshop download URL format
        // Note: This is a simplified implementation. Real Steam Workshop downloads
        // require authentication and use SteamCMD or the Steam API.
        let url = format!("https://steamcommunity.com/sharedfiles/filedetails/?id={}", workshop_id);
        
        info!(workshop_id, "Attempting to download from Steam Workshop");
        
        // For now, we'll need to use SteamCMD or a similar tool.
        // This is a placeholder implementation that would need to be extended
        // with actual Steam Workshop download logic.
        warn!(workshop_id, "Steam Workshop download not fully implemented - requires SteamCMD integration");
        
        Err(anyhow::anyhow!(
            "Steam Workshop downloads require SteamCMD integration. Workshop ID: {}",
            workshop_id
        ))
    }
}

#[async_trait]
impl Downloader for WorkshopDownloader {
    async fn download_workshop(&self, workshop_id: u64) -> anyhow::Result<PathBuf> {
        self.download_workshop_item(workshop_id).await
    }
    
    async fn download_zip(&self, url: &str) -> anyhow::Result<PathBuf> {
        // Delegate to ZIP downloader
        use crate::downloader::zip::ZipDownloader;
        let zip_downloader = ZipDownloader::new(self.temp_dir.clone()).await?;
        zip_downloader.download_zip(url).await
    }
}

