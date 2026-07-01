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
    pub async fn new(temp_dir: PathBuf, max_download_size_bytes: u64) -> anyhow::Result<Self> {
        tokio::fs::create_dir_all(&temp_dir).await?;
        
        Ok(Self {
            client: HttpClient::new(max_download_size_bytes)?,
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
        // Extract filename from URL and sanitize it
        let raw_filename = url
            .split('/')
            .last()
            .unwrap_or("download.zip");
        
        // Sanitize filename to prevent path traversal
        let filename = crate::utils::sanitize_filename(raw_filename);
        
        // If sanitization removed everything, use default
        let filename = if filename.is_empty() {
            "download.zip".to_string()
        } else {
            filename
        };
        
        let output_path = self.temp_dir.join(format!("{}-{}", Uuid::new_v4(), filename));
        
        info!(url = %url, path = %output_path.display(), "Downloading ZIP file");
        
        self.client.download_with_retry(url, &output_path).await?;
        
        Ok(output_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloader::test_lock::acquire_http_test_lock;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_download_zip_success() {
        let http = acquire_http_test_lock().await;
        let temp_dir = TempDir::new().unwrap();
        let downloader = ZipDownloader::new(temp_dir.path().to_path_buf(), 100 * 1024 * 1024).await.unwrap();
        let url = http.url("/test.zip");

        let result = downloader.download_zip(&url).await;
        assert!(result.is_ok());
        let downloaded_path = result.unwrap();
        assert!(downloaded_path.exists());
        assert!(downloaded_path.to_string_lossy().contains("test.zip"));

        let content = std::fs::read_to_string(&downloaded_path).unwrap();
        assert_eq!(content, "test file content");
    }

    #[tokio::test]
    async fn test_download_zip_with_path() {
        let http = acquire_http_test_lock().await;
        let temp_dir = TempDir::new().unwrap();
        let downloader = ZipDownloader::new(temp_dir.path().to_path_buf(), 100 * 1024 * 1024).await.unwrap();
        let url = http.url("/maps/custom/test_map.zip");

        let result = downloader.download_zip(&url).await;
        assert!(result.is_ok());
        let downloaded_path = result.unwrap();
        assert!(downloaded_path.exists());
        assert!(downloaded_path.to_string_lossy().contains("test_map.zip"));
    }

    #[tokio::test]
    async fn test_download_zip_without_extension() {
        let http = acquire_http_test_lock().await;
        let temp_dir = TempDir::new().unwrap();
        let downloader = ZipDownloader::new(temp_dir.path().to_path_buf(), 100 * 1024 * 1024).await.unwrap();
        let url = http.url("/download");

        let result = downloader.download_zip(&url).await;
        assert!(result.is_ok());
        let downloaded_path = result.unwrap();
        assert!(downloaded_path.exists());
        assert!(downloaded_path.to_string_lossy().contains("download"));
    }

    #[tokio::test]
    async fn test_download_zip_error() {
        let http = acquire_http_test_lock().await;
        let temp_dir = TempDir::new().unwrap();
        let downloader = ZipDownloader::new(temp_dir.path().to_path_buf(), 100 * 1024 * 1024).await.unwrap();
        let url = http.url("/error.zip");

        let result = downloader.download_zip(&url).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_download_workshop_not_supported() {
        let temp_dir = TempDir::new().unwrap();
        let downloader = ZipDownloader::new(temp_dir.path().to_path_buf(), 100 * 1024 * 1024).await.unwrap();

        let result = downloader.download_workshop(123456789).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Workshop downloads not supported"));
    }
}

