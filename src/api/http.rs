// SPDX-License-Identifier: GPL-3.0-only
use axum::Router;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;

use crate::api::handlers::ApiHandlers;
use crate::api::routes;
use crate::config::ConfigHandle;
use crate::map_installer::{MapInstallationService, PendingUpdatesState};
use crate::registry::Registry;

pub struct HttpServer {
    handlers: ApiHandlers,
    addr: SocketAddr,
}

impl HttpServer {
    pub fn new(
        registry: Arc<dyn Registry>,
        installer: Arc<MapInstallationService>,
        addr: SocketAddr,
        config: ConfigHandle,
        pending_updates: PendingUpdatesState,
    ) -> Self {
        Self {
            handlers: ApiHandlers::new(registry, installer, config, pending_updates),
            addr,
        }
    }

    pub fn router(handlers: Arc<ApiHandlers>) -> Router {
        routes::routes(handlers)
    }

    pub async fn serve(self) -> anyhow::Result<()> {
        let handlers = Arc::new(self.handlers);
        let app = Self::router(handlers);

        info!(addr = %self.addr, "Starting HTTP server");

        let listener = tokio::net::TcpListener::bind(&self.addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::response::ApiResponse;
    use crate::api::test_support::{setup_api_router, setup_api_router_with_config};
    use crate::config::{read_config, Config};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_health_endpoint() {
        let (app, _dirs) = setup_api_router().await;
        let response = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_health_endpoint_does_not_require_api_key() {
        let mut config = Config::default();
        config.backend_api_key = Some("secret".to_string());
        let (app, _handle, _dirs) = setup_api_router_with_config(config).await;
        let response = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_api_rejects_missing_or_wrong_bearer_token() {
        for authorization in [None, Some("Bearer wrong")] {
            let mut config = Config::default();
            config.backend_api_key = Some("secret".to_string());
            let (app, _handle, _dirs) = setup_api_router_with_config(config).await;
            let mut request = Request::get("/api/maps");
            if let Some(value) = authorization {
                request = request.header("authorization", value);
            }
            let response = app
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn test_api_accepts_correct_bearer_token() {
        let mut config = Config::default();
        config.backend_api_key = Some("secret".to_string());
        let (app, _handle, _dirs) = setup_api_router_with_config(config).await;
        let response = app
            .oneshot(
                Request::get("/api/maps")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_api_uses_hot_reloaded_bearer_token() {
        let mut config = Config::default();
        config.backend_api_key = Some("old-secret".to_string());
        let (app, handle, _dirs) = setup_api_router_with_config(config).await;

        let mut reloaded = (*read_config(&handle)).clone();
        reloaded.backend_api_key = Some("new-secret".to_string());
        *handle.write().unwrap() = Arc::new(reloaded);

        let old_response = app
            .clone()
            .oneshot(
                Request::get("/api/maps")
                    .header("authorization", "Bearer old-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(old_response.status(), StatusCode::UNAUTHORIZED);

        let new_response = app
            .oneshot(
                Request::get("/api/maps")
                    .header("authorization", "Bearer new-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(new_response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_list_maps_empty() {
        let (app, _dirs) = setup_api_router().await;
        let response = app
            .oneshot(Request::get("/api/maps").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_get_map_not_found_returns_json_error() {
        let (app, _dirs) = setup_api_router().await;
        let response = app
            .oneshot(
                Request::get("/api/maps/99999")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: ApiResponse<()> = serde_json::from_slice(&body).unwrap();
        assert!(!parsed.success);
        assert!(parsed.error.is_some());
    }

    #[tokio::test]
    async fn test_install_map_validation_error() {
        let (app, _dirs) = setup_api_router().await;
        let response = app
            .oneshot(
                Request::post("/api/maps/install")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"url":null,"workshop_id":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_uninstall_map_invalid_id() {
        let (app, _dirs) = setup_api_router().await;
        let response = app
            .oneshot(
                Request::post("/api/maps/uninstall/not-an-id")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_discover_maps_endpoint() {
        let (app, _dirs) = setup_api_router().await;
        let response = app
            .oneshot(
                Request::post("/api/maps/discover")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_compact_registry_endpoint() {
        let (app, _dirs) = setup_api_router().await;
        let response = app
            .oneshot(
                Request::post("/api/maps/compact")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_workshop_update_check_only() {
        let (app, _dirs) = setup_api_router().await;
        let response = app
            .oneshot(
                Request::post("/api/maps/workshop/update")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"check_only":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_modify_map_not_found() {
        let (app, _dirs) = setup_api_router().await;
        let response = app
            .oneshot(
                Request::patch("/api/maps/99999")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"field":"name","value":"Renamed"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_available_updates_endpoint_returns_cached_list() {
        use crate::api::handlers::ApiHandlers;
        use crate::config::init_handle;
        use crate::map_installer::{AvailableMapUpdate, MapInstallationService, PendingUpdatesState};
        use crate::registry::SourceKind;
        use crate::test_helpers;

        let (registry, dirs) = test_helpers::setup_test_dirs().await.unwrap();
        let paths = dirs.service_paths();
        let installer = Arc::new(
            MapInstallationService::new(
                Arc::clone(&registry),
                paths.addons_dir,
                paths.download_dir,
                100 * 1024 * 1024,
                1024 * 1024 * 1024,
                10000,
            )
            .await
            .unwrap(),
        );
        let pending = PendingUpdatesState::new();
        pending.replace_for_source(
            SourceKind::Workshop,
            vec![AvailableMapUpdate {
                name: "Campaign Foo".to_string(),
                map_id: 12,
                source_kind: SourceKind::Workshop,
                workshop_id: Some(123456),
            }],
        );
        let handlers = Arc::new(ApiHandlers::new(
            registry,
            installer,
            init_handle(Config::default()),
            pending,
        ));
        let app = HttpServer::router(handlers);
        let response = app
            .oneshot(
                Request::get("/api/maps/updates/available")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: ApiResponse<Vec<AvailableMapUpdate>> = serde_json::from_slice(&body).unwrap();
        assert!(parsed.success);
        let data = parsed.data.unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].map_id, 12);
        assert_eq!(data[0].name, "Campaign Foo");
        assert_eq!(data[0].source_kind, SourceKind::Workshop);
        let raw: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(raw["data"][0].get("workshop_id").is_none());
    }
}
