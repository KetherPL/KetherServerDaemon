// SPDX-License-Identifier: GPL-3.0-only
use axum::Router;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;

use crate::api::handlers::ApiHandlers;
use crate::api::routes;
use crate::map_installer::MapInstallationService;
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
    ) -> Self {
        Self {
            handlers: ApiHandlers::new(registry, installer),
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
    use crate::api::test_support::setup_api_router;
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
}
