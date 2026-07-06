// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use crate::config::{read_config, ConfigHandle};
use crate::registry::models::MapEntry;
use crate::sync::traits::{MapUpdate, SyncService};

#[derive(Debug, Clone)]
pub struct BackendSyncService {
    client: Client,
    config: ConfigHandle,
}

impl BackendSyncService {
    pub fn new(config: ConfigHandle) -> anyhow::Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")))
            .build()?;
        
        Ok(Self {
            client,
            config,
        })
    }
    
    fn build_request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let snapshot = read_config(&self.config);
        let url = format!("{}/{}", snapshot.backend_api_url.trim_end_matches('/'), path);
        let mut request = self.client.request(method, &url);

        if let Some(ref key) = snapshot.backend_api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        request
    }

    fn build_get_request(&self, path: &str) -> reqwest::RequestBuilder {
        self.build_request(reqwest::Method::GET, path)
    }

    fn build_post_request(&self, path: &str) -> reqwest::RequestBuilder {
        self.build_request(reqwest::Method::POST, path)
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
            .build_post_request("registry/sync")
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
            .build_get_request("registry/updates")
            .send()
            .await?;
        
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            warn!(status = %status, body = %text, "Failed to fetch updates from backend");
            return Err(anyhow::anyhow!(
                "Failed to fetch updates from backend: {} - {}",
                status,
                text
            ));
        }
        
        let updates_response: UpdatesResponse = response.json().await?;
        info!(count = updates_response.updates.len(), "Fetched updates from backend");
        
        Ok(updates_response.updates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{init_handle, Config};
    use crate::registry::models::SourceKind;
    use crate::sync::traits::SyncService;
    use axum::{Json, Router, routing::{get, post}};
    use axum::http::StatusCode;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;

    struct MockBackend {
        base_url: String,
        _handle: tokio::task::JoinHandle<()>,
    }

    async fn spawn_mock_backend(
        auth_header: Arc<Mutex<Option<String>>>,
        sync_status: StatusCode,
        updates_status: StatusCode,
    ) -> String {
        let sync_calls = Arc::new(AtomicUsize::new(0));
        let sync_calls_clone = Arc::clone(&sync_calls);
        let auth_clone = Arc::clone(&auth_header);

        let router = Router::new()
            .route(
                "/api/registry/sync",
                post({
                    let auth_clone = Arc::clone(&auth_clone);
                    move |headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| {
                        let auth_clone = Arc::clone(&auth_clone);
                        async move {
                            sync_calls_clone.fetch_add(1, Ordering::SeqCst);
                            if let Ok(mut stored) = auth_clone.try_lock() {
                                *stored = headers
                                    .get("authorization")
                                    .and_then(|v| v.to_str().ok())
                                    .map(String::from);
                            }
                            (sync_status, Json(body))
                        }
                    }
                }),
            )
            .route(
                "/api/registry/updates",
                get(move || async move {
                    if updates_status.is_success() {
                        (
                            updates_status,
                            Json(json!({
                                "updates": [{
                                    "action": "install",
                                    "map_id": "42",
                                    "map_entry": null
                                }]
                            })),
                        )
                    } else {
                        (updates_status, Json(json!({ "error": "backend down" })))
                    }
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let _mock = MockBackend {
            base_url: format!("http://{}", addr),
            _handle: handle,
        };

        format!("http://{}/api", addr)
    }

    fn sample_map_entry() -> MapEntry {
        MapEntry {
            id: 1,
            name: "Test".to_string(),
            source_url: "https://example.com".to_string(),
            source_kind: SourceKind::Other,
            workshop_id: None,
            installed_path: "test.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: None,
            checksum: None,
            checksum_kind: None,
        }
    }

    fn service_with_url(base_url: String, api_key: Option<String>) -> BackendSyncService {
        let mut config = Config::default();
        config.backend_api_url = base_url;
        config.backend_api_key = api_key;
        BackendSyncService::new(init_handle(config)).unwrap()
    }

    #[tokio::test]
    async fn test_sync_registry_sends_bearer_token() {
        let auth_header = Arc::new(Mutex::new(None));
        let base_url = spawn_mock_backend(
            Arc::clone(&auth_header),
            StatusCode::OK,
            StatusCode::OK,
        )
        .await;

        let service = service_with_url(base_url, Some("secret-token".to_string()));
        service.sync_registry(vec![sample_map_entry()]).await.unwrap();

        let stored = auth_header.lock().await.clone();
        assert_eq!(stored.as_deref(), Some("Bearer secret-token"));
    }

    #[tokio::test]
    async fn test_fetch_updates_returns_error_on_failure() {
        let auth_header = Arc::new(Mutex::new(None));
        let base_url = spawn_mock_backend(
            auth_header,
            StatusCode::OK,
            StatusCode::SERVICE_UNAVAILABLE,
        )
        .await;

        let service = service_with_url(base_url, None);
        let result = service.fetch_updates().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("503"));
    }

    #[tokio::test]
    async fn test_fetch_updates_parses_payload() {
        let auth_header = Arc::new(Mutex::new(None));
        let base_url = spawn_mock_backend(
            auth_header,
            StatusCode::OK,
            StatusCode::OK,
        )
        .await;

        let service = service_with_url(base_url, None);
        let updates = service.fetch_updates().await.unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].map_id, "42");
        assert_eq!(updates[0].action, "install");
    }
}
