// SPDX-License-Identifier: GPL-3.0-only
use reqwest::header::LOCATION;
use reqwest::redirect::Policy;
use reqwest::{Client, StatusCode, Url};
use std::time::Duration;
use tracing::{info, warn};

use crate::utils::validate_url_resolved;

const MAX_REDIRECTS: usize = 5;

pub struct HttpClient {
    client: Client,
    max_retries: u32,
    max_download_size: u64,
    enforce_ssrf: bool,
}

impl HttpClient {
    pub fn new(max_download_size: u64) -> anyhow::Result<Self> {
        Self::build(max_download_size, true)
    }

    /// Test helper: skip SSRF checks so loopback mock servers work.
    #[cfg(test)]
    pub fn new_insecure_for_tests(max_download_size: u64) -> anyhow::Result<Self> {
        Self::build(max_download_size, false)
    }

    fn build(max_download_size: u64, enforce_ssrf: bool) -> anyhow::Result<Self> {
        let client = Client::builder()
            .no_proxy()
            .pool_max_idle_per_host(0)
            // Large workshop VPKs can take well over 5 minutes on typical links.
            .timeout(Duration::from_secs(3600))
            .read_timeout(Duration::from_secs(120))
            .redirect(Policy::none())
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .user_agent(format!(
                "{}/{}",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            ))
            .build()?;

        Ok(Self {
            client,
            max_retries: 3,
            max_download_size,
            enforce_ssrf,
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

        Err(last_error.unwrap_or_else(|| {
            anyhow::anyhow!("Download failed after {} attempts", self.max_retries)
        }))
    }

    async fn download_once(
        &self,
        url: &str,
        output_path: &std::path::Path,
    ) -> anyhow::Result<()> {
        info!(url = %url, path = %output_path.display(), "Starting download");

        let mut current_url = url.to_string();
        let mut response = None;

        for hop in 0..=MAX_REDIRECTS {
            // Always re-validate redirect targets (DNS rebinding / private hop).
            // Initial URL may skip checks only for insecure test clients.
            if self.enforce_ssrf || hop > 0 {
                validate_url_resolved(&current_url).await?;
            }

            let candidate = self.client.get(&current_url).send().await?;
            let status = candidate.status();

            if status.is_redirection() {
                if hop == MAX_REDIRECTS {
                    return Err(anyhow::anyhow!(
                        "Too many redirects while downloading {url} (limit {MAX_REDIRECTS})"
                    ));
                }
                let location = candidate
                    .headers()
                    .get(LOCATION)
                    .ok_or_else(|| {
                        anyhow::anyhow!("Redirect response missing Location header")
                    })?
                    .to_str()
                    .map_err(|_| anyhow::anyhow!("Redirect Location header is not valid UTF-8"))?;

                let next = Url::parse(&current_url)
                    .unwrap_or_else(|_| Url::parse("http://invalid.invalid").unwrap())
                    .join(location)
                    .map_err(|e| anyhow::anyhow!("Invalid redirect Location '{location}': {e}"))?;
                current_url = next.to_string();
                continue;
            }

            response = Some(candidate);
            break;
        }

        let response = response.ok_or_else(|| anyhow::anyhow!("Download produced no response"))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(anyhow::anyhow!("Download failed with status 404 Not Found"));
        }
        response.error_for_status_ref()?;

        if let Some(content_length) = response.content_length()
            && content_length > self.max_download_size
        {
            return Err(anyhow::anyhow!(
                "File size {} exceeds maximum download size {} bytes",
                content_length,
                self.max_download_size
            ));
        }

        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;

        let mut stream = response.bytes_stream();
        let mut downloaded: u64 = 0;
        let mut file = tokio::fs::File::create(output_path).await?;

        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(chunk) => chunk,
                Err(error) => {
                    let _ = tokio::fs::remove_file(output_path).await;
                    return Err(anyhow::anyhow!(
                        "Download stream failed after {} bytes: {}",
                        downloaded,
                        error
                    ));
                }
            };
            downloaded += chunk.len() as u64;

            if downloaded > self.max_download_size {
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
    use axum::{
        Router,
        body::Body,
        http::{HeaderValue, StatusCode as AxumStatus},
        response::Response,
        routing::get,
    };
    use tempfile::TempDir;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn test_download_success() {
        let http = acquire_http_test_lock().await;
        let client = HttpClient::new_insecure_for_tests(100 * 1024 * 1024).unwrap();
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
        let client = HttpClient::new_insecure_for_tests(100 * 1024 * 1024).unwrap();
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
        let client = HttpClient::new_insecure_for_tests(100 * 1024 * 1024).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("downloaded.zip");
        let url = http.url("/internalerror.zip");

        let result = client.download_with_retry(&url, &output_path).await;
        assert!(result.is_err());
        assert!(!output_path.exists());
    }

    #[tokio::test]
    async fn test_download_invalid_url() {
        let client = HttpClient::new_insecure_for_tests(100 * 1024 * 1024).unwrap();
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
        let client = HttpClient::new_insecure_for_tests(100 * 1024 * 1024).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("large.zip");
        let url = http.url("/large.zip");

        let result = client.download_with_retry(&url, &output_path).await;
        assert!(result.is_ok());
        assert!(output_path.exists());

        let content = std::fs::read_to_string(&output_path).unwrap();
        assert_eq!(content.len(), 32 * 1024);
    }

    #[tokio::test]
    async fn download_rejects_private_literal_url() {
        let client = HttpClient::new(1024 * 1024).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("out.bin");
        let result = client
            .download_with_retry("http://127.0.0.1/secret", &output_path)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn download_rejects_redirect_to_private_ip() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/redir",
            get(|| async {
                Response::builder()
                    .status(AxumStatus::FOUND)
                    .header(
                        axum::http::header::LOCATION,
                        HeaderValue::from_static("http://192.168.1.10/secret"),
                    )
                    .body(Body::empty())
                    .unwrap()
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // Allow the loopback first hop (test client), but redirect re-validation must fail.
        let client = HttpClient::new_insecure_for_tests(1024 * 1024).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("out.bin");
        let url = format!("http://{addr}/redir");
        let result = client.download_with_retry(&url, &output_path).await;
        assert!(result.is_err());
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("private")
                || message.contains("internal")
                || message.contains("localhost"),
            "unexpected error: {message}"
        );
        assert!(!output_path.exists());
    }
}
