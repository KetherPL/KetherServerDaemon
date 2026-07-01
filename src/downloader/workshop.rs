// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::OnceCell;
use tracing::info;
use uuid::Uuid;
use crate::downloader::{
    client::HttpClient,
    steam::{SteamConnection, SteamError, WorkshopFileDetails},
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
            client: HttpClient::new(100 * 1024 * 1024)?, // Default 100MB, should be passed from config
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
        
        let steam = self.get_steam_connection().await?;
        let hcontent = steam
            .get_hcontent_from_workshop_id(workshop_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get hcontent: {}", e))?;

        self.download_from_hcontent(workshop_id, hcontent).await
    }

    /// Download a workshop item when hcontent is already known (avoids a GetDetails round-trip).
    pub async fn download_from_details(
        &self,
        detail: &WorkshopFileDetails,
    ) -> anyhow::Result<PathBuf> {
        self.download_from_hcontent(detail.workshop_id, detail.hcontent)
            .await
    }

    /// Batch-fetch workshop metadata via Steam.
    pub async fn get_workshop_file_details(
        &self,
        workshop_ids: &[u64],
    ) -> anyhow::Result<Vec<WorkshopFileDetails>> {
        let steam = self.get_steam_connection().await?;
        steam
            .get_workshop_file_details(workshop_ids)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get workshop file details: {}", e))
    }

    async fn download_from_hcontent(
        &self,
        workshop_id: u64,
        hcontent: u64,
    ) -> anyhow::Result<PathBuf> {
        let steam = self.get_steam_connection().await?;
        
        let download_url = steam
            .get_download_url(hcontent)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get download URL: {}", e))?;
        
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloader::test_lock::acquire_http_test_lock;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_download_zip_delegation() {
        let http = acquire_http_test_lock().await;
        let temp_dir = TempDir::new().unwrap();
        let downloader = WorkshopDownloader::new(temp_dir.path().to_path_buf()).unwrap();
        let url = http.url("/workshop.zip");

        let result = downloader.download_zip(&url).await;
        assert!(result.is_ok());
        let downloaded_path = result.unwrap();
        assert!(downloaded_path.exists());

        let content = std::fs::read_to_string(&downloaded_path).unwrap();
        assert_eq!(content, "zip content");
    }

    #[tokio::test]
    async fn test_download_workshop_requires_steam_connection() {
        // Note: This test will fail because it requires a real Steam connection.
        // In a real scenario, we would either:
        // 1. Mock the SteamConnection
        // 2. Use integration tests with actual Steam API
        // 3. Skip this test in CI
        //
        // For now, we'll test that the error is handled gracefully
        
        let temp_dir = TempDir::new().unwrap();
        let downloader = WorkshopDownloader::new(temp_dir.path().to_path_buf()).unwrap();
        
        // This will fail because we can't connect to Steam in unit tests
        // The exact error depends on network/Steam API availability
        let result = downloader.download_workshop(123456789).await;
        assert!(result.is_err());
        
        // Verify the error message indicates Steam connection issue
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("Steam connection") || 
            error_msg.contains("Failed to establish") ||
            error_msg.contains("hcontent") ||
            error_msg.contains("network")
        );
    }

    // Note: To properly test Steam integration, we would need:
    // 1. A mock implementation of SteamConnection
    // 2. Or integration tests with actual Steam API access
    // 3. Or use dependency injection to allow injecting a mock SteamConnection
}

