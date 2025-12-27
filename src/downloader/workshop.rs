// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::OnceCell;
use tracing::info;
use uuid::Uuid;
use crate::downloader::{
    client::HttpClient,
    steam::{SteamConnection, SteamError},
    traits::Downloader,
};

pub struct WorkshopDownloader {
    client: HttpClient,
    temp_dir: PathBuf,
    steam_connection: Arc<OnceCell<Result<SteamConnection, SteamError>>>,
}

impl WorkshopDownloader {
    pub fn new(temp_dir: PathBuf) -> anyhow::Result<Self> {
        Ok(Self {
            client: HttpClient::new()?,
            temp_dir,
            steam_connection: Arc::new(OnceCell::new()),
        })
    }
    
    /// Get or initialize Steam connection
    async fn get_steam_connection(&self) -> anyhow::Result<&SteamConnection> {
        self.steam_connection
            .get_or_init(|| async { SteamConnection::new().await })
            .await
            .as_ref()
            .map_err(|e| anyhow::anyhow!("Failed to establish Steam connection: {}", e))
    }
    
    async fn download_workshop_item(&self, workshop_id: u64) -> anyhow::Result<PathBuf> {
        info!(workshop_id, "Starting Steam Workshop download");
        
        // Get Steam connection (lazy-initialized, reused across calls)
        let steam = self.get_steam_connection().await?;
        
        // Step 1: Get hcontent from workshop ID
        let hcontent = steam
            .get_hcontent_from_workshop_id(workshop_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get hcontent: {}", e))?;
        
        // Step 2: Get download URL from hcontent
        let download_url = steam
            .get_download_url(hcontent)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get download URL: {}", e))?;
        
        // Step 3: Download file using existing HTTP client
        let filename = download_url
            .split('/')
            .last()
            .unwrap_or("workshop_download")
            .split('?')
            .next()
            .unwrap_or("workshop_download");
        
        let output_path = self.temp_dir.join(format!("{}-{}", Uuid::new_v4(), filename));
        
        info!(
            workshop_id,
            url = %download_url,
            path = %output_path.display(),
            "Downloading workshop file"
        );
        
        self.client
            .download_with_retry(&download_url, &output_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to download file: {}", e))?;
        
        info!(
            workshop_id,
            path = %output_path.display(),
            "Workshop download completed"
        );
        
        Ok(output_path)
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

