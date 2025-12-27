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

