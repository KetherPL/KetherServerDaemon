// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use std::path::PathBuf;
use tracing::info;
use crate::downloader::{client::HttpClient, traits::Downloader};
use uuid::Uuid;

pub struct ZipDownloader {
    client: HttpClient,
    temp_dir: PathBuf,
}

impl ZipDownloader {
    pub async fn new(temp_dir: PathBuf) -> anyhow::Result<Self> {
        tokio::fs::create_dir_all(&temp_dir).await?;
        
        Ok(Self {
            client: HttpClient::new()?,
            temp_dir,
        })
    }
}

#[async_trait]
impl Downloader for ZipDownloader {
    async fn download_workshop(&self, _workshop_id: u64) -> anyhow::Result<PathBuf> {
        Err(anyhow::anyhow!("Workshop downloads not supported by ZipDownloader"))
    }
    
    async fn download_zip(&self, url: &str) -> anyhow::Result<PathBuf> {
        let filename = url
            .split('/')
            .last()
            .unwrap_or("download.zip");
        
        let output_path = self.temp_dir.join(format!("{}-{}", Uuid::new_v4(), filename));
        
        info!(url = %url, path = %output_path.display(), "Downloading ZIP file");
        
        self.client.download_with_retry(url, &output_path).await?;
        
        Ok(output_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::ServerGuard;
    use tempfile::TempDir;

    async fn setup_mock_server() -> (ServerGuard, String) {
        let server = mockito::Server::new_async().await;
        let base_url = server.url();
        (server, base_url)
    }

    #[tokio::test]
    async fn test_download_zip_success() {
        let (mut server, base_url) = setup_mock_server().await;
        let temp_dir = TempDir::new().unwrap();
        let downloader = ZipDownloader::new(temp_dir.path().to_path_buf()).await.unwrap();
        
        let mock = server.mock("GET", "/test.zip")
            .with_status(200)
            .with_body("zip file content")
            .create_async()
            .await;
        
        let url = format!("{}/test.zip", base_url);
        let result = downloader.download_zip(&url).await;
        
        assert!(result.is_ok());
        let downloaded_path = result.unwrap();
        assert!(downloaded_path.exists());
        assert!(downloaded_path.to_string_lossy().contains("test.zip"));
        
        let content = std::fs::read_to_string(&downloaded_path).unwrap();
        assert_eq!(content, "zip file content");
        
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_download_zip_with_path() {
        let (mut server, base_url) = setup_mock_server().await;
        let temp_dir = TempDir::new().unwrap();
        let downloader = ZipDownloader::new(temp_dir.path().to_path_buf()).await.unwrap();
        
        let mock = server.mock("GET", "/maps/custom/test_map.zip")
            .with_status(200)
            .with_body("map content")
            .create_async()
            .await;
        
        let url = format!("{}/maps/custom/test_map.zip", base_url);
        let result = downloader.download_zip(&url).await;
        
        assert!(result.is_ok());
        let downloaded_path = result.unwrap();
        assert!(downloaded_path.exists());
        assert!(downloaded_path.to_string_lossy().contains("test_map.zip"));
        
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_download_zip_without_extension() {
        let (mut server, base_url) = setup_mock_server().await;
        let temp_dir = TempDir::new().unwrap();
        let downloader = ZipDownloader::new(temp_dir.path().to_path_buf()).await.unwrap();
        
        let mock = server.mock("GET", "/download")
            .with_status(200)
            .with_body("content")
            .create_async()
            .await;
        
        let url = format!("{}/download", base_url);
        let result = downloader.download_zip(&url).await;
        
        assert!(result.is_ok());
        let downloaded_path = result.unwrap();
        assert!(downloaded_path.exists());
        // Should default to "download.zip" when no filename in URL
        assert!(downloaded_path.to_string_lossy().contains("download"));
        
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_download_zip_error() {
        let (mut server, base_url) = setup_mock_server().await;
        let temp_dir = TempDir::new().unwrap();
        let downloader = ZipDownloader::new(temp_dir.path().to_path_buf()).await.unwrap();
        
        let mock = server.mock("GET", "/error.zip")
            .with_status(404)
            .create_async()
            .await;
        
        let url = format!("{}/error.zip", base_url);
        let result = downloader.download_zip(&url).await;
        
        assert!(result.is_err());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_download_workshop_not_supported() {
        let temp_dir = TempDir::new().unwrap();
        let downloader = ZipDownloader::new(temp_dir.path().to_path_buf()).await.unwrap();
        
        let result = downloader.download_workshop(123456789).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Workshop downloads not supported"));
    }
}

