// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};
use uuid::Uuid;
use crate::downloader::{
    client::HttpClient,
    steam::{SteamConnection, SteamError, WorkshopFileDetails},
    traits::Downloader,
};

const STEAM_PUBLISHED_FILE_DETAILS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";

pub struct WorkshopDownloader {
    client: HttpClient,
    temp_dir: PathBuf,
    max_download_size_bytes: u64,
    steam_connection: Arc<Mutex<Option<SteamConnection>>>,
}

impl WorkshopDownloader {
    pub fn new(temp_dir: PathBuf, max_download_size_bytes: u64) -> anyhow::Result<Self> {
        Ok(Self {
            client: HttpClient::new(max_download_size_bytes)?,
            temp_dir,
            max_download_size_bytes,
            steam_connection: Arc::new(Mutex::new(None)),
        })
    }

    async fn connect_steam(&self) -> anyhow::Result<SteamConnection> {
        SteamConnection::new()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to establish Steam connection: {}", e))
    }
    
    /// Get or initialize Steam connection (reconnects when missing).
    async fn get_steam_connection(&self) -> anyhow::Result<SteamConnection> {
        let mut guard = self.steam_connection.lock().await;
        if guard.is_none() {
            *guard = Some(self.connect_steam().await?);
        }
        Ok(guard.as_ref().unwrap().clone())
    }

    async fn reset_steam_connection(&self) -> anyhow::Result<SteamConnection> {
        warn!("Resetting Steam connection after request failure");
        let connection = self.connect_steam().await?;
        let mut guard = self.steam_connection.lock().await;
        *guard = Some(connection.clone());
        Ok(connection)
    }
    
    async fn download_workshop_item(&self, workshop_id: u64) -> anyhow::Result<PathBuf> {
        info!(workshop_id, "Starting Steam Workshop download");

        let details = self.get_workshop_file_details(&[workshop_id]).await?;
        let detail = details
            .into_iter()
            .find(|d| d.workshop_id == workshop_id)
            .ok_or_else(|| anyhow::anyhow!("Workshop item {workshop_id} not found on Steam"))?;

        self.download_from_details(&detail).await
    }

    /// Download a workshop item when metadata is already known.
    pub async fn download_from_details(
        &self,
        detail: &WorkshopFileDetails,
    ) -> anyhow::Result<PathBuf> {
        let download_url = match self.resolve_download_url(detail).await? {
            Some(url) => url,
            None => {
                return Err(anyhow::anyhow!(
                    "No download URL available for workshop item {}",
                    detail.workshop_id
                ));
            }
        };

        self.download_from_url(detail.workshop_id, &download_url)
            .await
    }

    async fn resolve_download_url(
        &self,
        detail: &WorkshopFileDetails,
    ) -> anyhow::Result<Option<String>> {
        if let Some(url) = detail.file_url.as_deref() {
            info!(
                workshop_id = detail.workshop_id,
                url = %url,
                "Using file_url from workshop metadata"
            );
            return Ok(Some(url.to_string()));
        }

        if let Some(url) = self.fetch_file_url_via_web_api(detail.workshop_id).await? {
            info!(
                workshop_id = detail.workshop_id,
                url = %url,
                "Using file_url from Steam Web API"
            );
            return Ok(Some(url));
        }

        if detail.hcontent == 0 {
            return Ok(None);
        }

        warn!(
            workshop_id = detail.workshop_id,
            hcontent = detail.hcontent,
            "No direct file_url; falling back to Steam UFS hcontent lookup"
        );

        let steam = self.get_steam_connection().await?;
        match steam.get_download_url(detail.hcontent).await {
            Ok(url) => Ok(Some(url)),
            Err(error) if error.is_timeout() => {
                self.reset_steam_connection()
                    .await?
                    .get_download_url(detail.hcontent)
                    .await
                    .map(Some)
                    .map_err(|e| anyhow::anyhow!("Failed to get download URL: {}", e))
            }
            Err(error) => Err(anyhow::anyhow!("Failed to get download URL: {}", error)),
        }
    }

    async fn fetch_file_url_via_web_api(
        &self,
        workshop_id: u64,
    ) -> anyhow::Result<Option<String>> {
        #[derive(Deserialize)]
        struct WebApiResponse {
            response: WebApiResponseBody,
        }

        #[derive(Deserialize)]
        struct WebApiResponseBody {
            publishedfiledetails: Vec<WebApiPublishedFile>,
        }

        #[derive(Deserialize)]
        struct WebApiPublishedFile {
            file_url: Option<String>,
        }

        let body = format!("itemcount=1&publishedfileids%5B0%5D={workshop_id}");
        let response = self
            .client
            .client()
            .post(STEAM_PUBLISHED_FILE_DETAILS_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Steam Web API request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Steam Web API returned HTTP {}",
                response.status()
            ));
        }

        let body: WebApiResponse = response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse Steam Web API response: {}", e))?;

        Ok(body
            .response
            .publishedfiledetails
            .into_iter()
            .next()
            .and_then(|item| item.file_url)
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty()))
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

    async fn download_from_url(
        &self,
        workshop_id: u64,
        download_url: &str,
    ) -> anyhow::Result<PathBuf> {
        let filename = download_url
            .trim_end_matches('/')
            .split('/')
            .last()
            .filter(|segment| !segment.is_empty())
            .unwrap_or_else(|| "workshop_download")
            .split('?')
            .next()
            .unwrap_or("workshop_download");

        let filename = if filename.is_empty() || filename == "workshop_download" {
            format!("{workshop_id}.vpk")
        } else if !filename.contains('.') {
            format!("{filename}.vpk")
        } else {
            filename.to_string()
        };
        
        let output_path = self.temp_dir.join(format!("{}-{}", Uuid::new_v4(), filename));
        
        info!(
            workshop_id,
            url = %download_url,
            path = %output_path.display(),
            "Downloading workshop file"
        );
        
        self.client
            .download_with_retry(download_url, &output_path)
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
        let zip_downloader =
            ZipDownloader::new(self.temp_dir.clone(), self.max_download_size_bytes).await?;
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
        let downloader = WorkshopDownloader::new(temp_dir.path().to_path_buf(), 100 * 1024 * 1024).unwrap();
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
        let temp_dir = TempDir::new().unwrap();
        let downloader = WorkshopDownloader::new(temp_dir.path().to_path_buf(), 100 * 1024 * 1024).unwrap();
        
        let result = downloader.download_workshop(123456789).await;
        assert!(result.is_err());
        
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("Steam connection") || 
            error_msg.contains("Failed to establish") ||
            error_msg.contains("hcontent") ||
            error_msg.contains("network") ||
            error_msg.contains("not found")
        );
    }

    #[tokio::test]
    async fn test_fetch_file_url_via_web_api_returns_url_for_known_item() {
        let temp_dir = TempDir::new().unwrap();
        let downloader = WorkshopDownloader::new(temp_dir.path().to_path_buf(), 100 * 1024 * 1024).unwrap();

        let url = downloader
            .fetch_file_url_via_web_api(3726403340)
            .await
            .unwrap()
            .unwrap_or_default();

        assert!(url.contains("steamusercontent.com"));
        assert!(url.contains("15796922369319871036"));
    }
}
