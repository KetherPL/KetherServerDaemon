// SPDX-License-Identifier: GPL-3.0-only
use reqwest::Client;
use std::time::Duration;
use tracing::{info, warn};

pub struct HttpClient {
    client: Client,
    max_retries: u32,
    max_download_size: u64,
}

impl HttpClient {
    pub fn new(max_download_size: u64) -> anyhow::Result<Self> {
        let client = Client::builder()
            .no_proxy()
            .pool_max_idle_per_host(0)
            .timeout(Duration::from_secs(300)) // 5 minute timeout for large downloads
            .user_agent("KetherServerDaemon/0.0.1")
            .build()?;
        
        Ok(Self {
            client,
            max_retries: 3,
            max_download_size,
        })
    }
    
    pub async fn download_with_retry(
        &self,
        url: &str,
        output_path: &std::path::Path,
    ) -> anyhow::Result<()> {
        let mut last_error = None;
        
        for attempt in 1..=self.max_retries {
            match self.download_once(url, output_path).await {
                Ok(()) => {
                    if attempt > 1 {
                        info!(url = %url, attempt, "Download succeeded after retry");
                    }
                    return Ok(());
                }
                Err(e) => {
                    warn!(url = %url, attempt, error = %e, "Download attempt failed");
                    last_error = Some(e);
                    if attempt < self.max_retries {
                        tokio::time::sleep(Duration::from_secs(2_u64.pow(attempt))).await;
                    }
                }
            }
        }
        
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Download failed after {} attempts", self.max_retries)))
    }
    
    async fn download_once(
        &self,
        url: &str,
        output_path: &std::path::Path,
    ) -> anyhow::Result<()> {
        info!(url = %url, path = %output_path.display(), "Starting download");
        
        let response = self.client.get(url).send().await?;
        response.error_for_status_ref()?;
        
        // Check Content-Length header if available
        if let Some(content_length) = response.content_length() {
            if content_length > self.max_download_size {
                return Err(anyhow::anyhow!(
                    "File size {} exceeds maximum download size {} bytes",
                    content_length,
                    self.max_download_size
                ));
            }
        }
        
        // Stream download to check size during transfer
        use tokio::io::AsyncWriteExt;
        use futures_util::StreamExt;
        
        let mut stream = response.bytes_stream();
        let mut downloaded: u64 = 0;
        let mut file = tokio::fs::File::create(output_path).await?;
        
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result?;
            downloaded += chunk.len() as u64;
            
            if downloaded > self.max_download_size {
                // Try to clean up partial file
                let _ = tokio::fs::remove_file(output_path).await;
                return Err(anyhow::anyhow!(
                    "Download size {} exceeds maximum download size {} bytes",
                    downloaded,
                    self.max_download_size
                ));
            }
            
            file.write_all(&chunk).await?;
        }
        
        file.flush().await?;

        if downloaded == 0 {
            let _ = tokio::fs::remove_file(output_path).await;
            return Err(anyhow::anyhow!("Download completed with 0 bytes"));
        }
        
        info!(url = %url, path = %output_path.display(), size = downloaded, "Download completed");
        Ok(())
    }
    
    pub fn client(&self) -> &Client {
        &self.client
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new(100 * 1024 * 1024).expect("Failed to create HTTP client") // 100MB default
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloader::test_lock::acquire_http_test_lock;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_download_success() {
        let http = acquire_http_test_lock().await;
        let client = HttpClient::new(100 * 1024 * 1024).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("downloaded.zip");
        let url = http.url("/test.zip");

        let result = client.download_with_retry(&url, &output_path).await;
        assert!(result.is_ok());
        assert!(output_path.exists());

        let content = std::fs::read_to_string(&output_path).unwrap();
        assert_eq!(content, "test file content");
    }

    #[tokio::test]
    async fn test_download_404_error() {
        let http = acquire_http_test_lock().await;
        let client = HttpClient::new(100 * 1024 * 1024).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("downloaded.zip");
        let url = http.url("/notfound.zip");

        let result = client.download_with_retry(&url, &output_path).await;
        assert!(result.is_err());
        assert!(!output_path.exists());
    }

    #[tokio::test]
    async fn test_download_500_error() {
        let http = acquire_http_test_lock().await;
        let client = HttpClient::new(100 * 1024 * 1024).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("downloaded.zip");
        let url = http.url("/internalerror.zip");

        let result = client.download_with_retry(&url, &output_path).await;
        assert!(result.is_err());
        assert!(!output_path.exists());
    }

    #[tokio::test]
    async fn test_download_invalid_url() {
        let client = HttpClient::new(100 * 1024 * 1024).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("downloaded.zip");
        let invalid_url = "not-a-valid-url";

        let result = client.download_with_retry(invalid_url, &output_path).await;
        assert!(result.is_err());
        assert!(!output_path.exists());
    }

    #[tokio::test]
    async fn test_download_large_file() {
        let http = acquire_http_test_lock().await;
        let client = HttpClient::new(100 * 1024 * 1024).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("large.zip");
        let url = http.url("/large.zip");

        let result = client.download_with_retry(&url, &output_path).await;
        assert!(result.is_ok());
        assert!(output_path.exists());

        let content = std::fs::read_to_string(&output_path).unwrap();
        assert_eq!(content.len(), 32 * 1024);
    }
}

