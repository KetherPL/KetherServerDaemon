// SPDX-License-Identifier: GPL-3.0-only
use reqwest::Client;
use std::time::Duration;
use tracing::{info, warn};

pub struct HttpClient {
    client: Client,
    max_retries: u32,
}

impl HttpClient {
    pub fn new() -> anyhow::Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(300)) // 5 minute timeout for large downloads
            .user_agent("KetherServerDaemon/0.0.1")
            .build()?;
        
        Ok(Self {
            client,
            max_retries: 3,
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
        
        let content = response.bytes().await?;
        tokio::fs::write(output_path, content).await?;
        
        info!(url = %url, path = %output_path.display(), "Download completed");
        Ok(())
    }
    
    pub fn client(&self) -> &Client {
        &self.client
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new().expect("Failed to create HTTP client")
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
    async fn test_download_success() {
        let (mut server, base_url) = setup_mock_server().await;
        let client = HttpClient::new().unwrap();
        
        let mock = server.mock("GET", "/test.zip")
            .with_status(200)
            .with_body("test file content")
            .create_async()
            .await;
        
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("downloaded.zip");
        let url = format!("{}/test.zip", base_url);
        
        let result = client.download_with_retry(&url, &output_path).await;
        assert!(result.is_ok());
        assert!(output_path.exists());
        
        let content = std::fs::read_to_string(&output_path).unwrap();
        assert_eq!(content, "test file content");
        
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_download_404_error() {
        let (mut server, base_url) = setup_mock_server().await;
        let client = HttpClient::new().unwrap();
        
        let mock = server.mock("GET", "/notfound.zip")
            .with_status(404)
            .create_async()
            .await;
        
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("downloaded.zip");
        let url = format!("{}/notfound.zip", base_url);
        
        let result = client.download_with_retry(&url, &output_path).await;
        assert!(result.is_err());
        assert!(!output_path.exists());
        
        // Should have retried max_retries times (3 times by default)
        mock.expect_at_least(3).assert_async().await;
    }

    #[tokio::test]
    async fn test_download_500_error() {
        let (mut server, base_url) = setup_mock_server().await;
        let client = HttpClient::new().unwrap();
        
        let mock = server.mock("GET", "/error.zip")
            .with_status(500)
            .create_async()
            .await;
        
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("downloaded.zip");
        let url = format!("{}/error.zip", base_url);
        
        let result = client.download_with_retry(&url, &output_path).await;
        assert!(result.is_err());
        assert!(!output_path.exists());
        
        // Should have retried max_retries times
        mock.expect_at_least(3).assert_async().await;
    }

    #[tokio::test]
    async fn test_download_retry_success_after_failure() {
        let (mut server, base_url) = setup_mock_server().await;
        let client = HttpClient::new().unwrap();
        
        // First two attempts fail, third succeeds
        let mock_fail1 = server.mock("GET", "/retry.zip")
            .with_status(500)
            .expect(1)
            .create_async()
            .await;
        
        let mock_fail2 = server.mock("GET", "/retry.zip")
            .with_status(500)
            .expect(1)
            .create_async()
            .await;
        
        let mock_success = server.mock("GET", "/retry.zip")
            .with_status(200)
            .with_body("successful content")
            .expect(1)
            .create_async()
            .await;
        
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("downloaded.zip");
        let url = format!("{}/retry.zip", base_url);
        
        let result = client.download_with_retry(&url, &output_path).await;
        assert!(result.is_ok());
        assert!(output_path.exists());
        
        let content = std::fs::read_to_string(&output_path).unwrap();
        assert_eq!(content, "successful content");
        
        mock_fail1.assert_async().await;
        mock_fail2.assert_async().await;
        mock_success.assert_async().await;
    }

    #[tokio::test]
    async fn test_download_invalid_url() {
        let client = HttpClient::new().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("downloaded.zip");
        let invalid_url = "not-a-valid-url";
        
        let result = client.download_with_retry(invalid_url, &output_path).await;
        assert!(result.is_err());
        assert!(!output_path.exists());
    }

    #[tokio::test]
    async fn test_download_large_file() {
        let (mut server, base_url) = setup_mock_server().await;
        let client = HttpClient::new().unwrap();
        
        let large_content = "x".repeat(1024 * 1024); // 1MB
        let mock = server.mock("GET", "/large.zip")
            .with_status(200)
            .with_body(&large_content)
            .create_async()
            .await;
        
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("large.zip");
        let url = format!("{}/large.zip", base_url);
        
        let result = client.download_with_retry(&url, &output_path).await;
        assert!(result.is_ok());
        assert!(output_path.exists());
        
        let content = std::fs::read_to_string(&output_path).unwrap();
        assert_eq!(content.len(), large_content.len());
        
        mock.assert_async().await;
    }
}

