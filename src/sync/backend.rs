// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use crate::registry::models::MapEntry;
use crate::sync::traits::{MapUpdate, SyncService};

#[derive(Debug, Clone)]
pub struct BackendSyncService {
    client: Client,
    base_url: String,
    api_key: Option<String>,
}

impl BackendSyncService {
    pub fn new(base_url: String, api_key: Option<String>) -> anyhow::Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("KetherServerDaemon/0.0.1")
            .build()?;
        
        Ok(Self {
            client,
            base_url,
            api_key,
        })
    }
    
    fn build_request(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), path);
        let mut request = self.client.get(&url);
        
        if let Some(ref key) = self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }
        
        request
    }
    
    fn build_post_request(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), path);
        let mut request = self.client.post(&url);
        
        if let Some(ref key) = self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }
        
        request
    }
}

#[derive(Serialize)]
struct SyncRequest {
    maps: Vec<MapEntry>,
}

#[derive(Deserialize)]
struct UpdatesResponse {
    updates: Vec<MapUpdate>,
}

#[async_trait]
impl SyncService for BackendSyncService {
    async fn sync_registry(&self, entries: Vec<MapEntry>) -> anyhow::Result<()> {
        info!(count = entries.len(), "Syncing registry to backend");
        
        let request = SyncRequest { maps: entries };
        let response = self
            .build_post_request("api/registry/sync")
            .json(&request)
            .send()
            .await?;
        
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            error!(status = %status, body = %text, "Backend sync failed");
            return Err(anyhow::anyhow!("Backend sync failed: {} - {}", status, text));
        }
        
        info!("Registry sync completed successfully");
        Ok(())
    }
    
    async fn fetch_updates(&self) -> anyhow::Result<Vec<MapUpdate>> {
        info!("Fetching updates from backend");
        
        let response = self
            .build_request("api/registry/updates")
            .send()
            .await?;
        
        if !response.status().is_success() {
            let status = response.status();
            warn!(status = %status, "Failed to fetch updates from backend");
            return Ok(Vec::new()); // Return empty on error to allow continuation
        }
        
        let updates_response: UpdatesResponse = response.json().await?;
        info!(count = updates_response.updates.len(), "Fetched updates from backend");
        
        Ok(updates_response.updates)
    }
}

