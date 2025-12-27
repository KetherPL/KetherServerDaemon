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

