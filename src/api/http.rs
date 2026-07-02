// SPDX-License-Identifier: GPL-3.0-only
use axum::{
    extract::Path,
    routing::{get, post},
    Json, Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;

use crate::api::error::ApiError;
use crate::api::handlers::{
    ApiHandlers, ApiResponse, DiscoverRequest, InstallMapRequest, ModifyMapRequest,
    UpdateWorkshopRequest,
};
use crate::registry::{MapEntry, Registry};
use crate::map_installer::{
    CompactReport, DiscoveryReport, MapInstallationService, WorkshopUpdateReport,
};

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
        Router::new()
            .route("/health", get(health_handler))
            .route("/api/maps/install", post(install_map_handler))
            .route("/api/maps/uninstall/{id}", post(uninstall_map_handler))
            .route("/api/maps/workshop/update", post(update_workshop_handler))
            .route("/api/maps/discover", post(discover_handler))
            .route("/api/maps/compact", post(compact_handler))
            .route("/api/maps", get(list_maps_handler))
            .route("/api/maps/{id}", get(get_map_handler).patch(modify_map_handler))
            .with_state(handlers)
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

async fn health_handler() -> Json<ApiResponse<&'static str>> {
    Json(ApiResponse::success("ok"))
}

async fn list_maps_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
) -> Result<Json<ApiResponse<Vec<MapEntry>>>, ApiError> {
    handlers.list_maps().await
}

async fn get_map_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<MapEntry>>, ApiError> {
    handlers.get_map(Path(id)).await
}

async fn install_map_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Json(request): Json<InstallMapRequest>,
) -> Result<Json<ApiResponse<u64>>, ApiError> {
    handlers.install_map(Json(request)).await
}

async fn uninstall_map_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    handlers.uninstall_map(Path(id)).await
}

async fn update_workshop_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Json(request): Json<UpdateWorkshopRequest>,
) -> Result<Json<ApiResponse<WorkshopUpdateReport>>, ApiError> {
    handlers.update_workshop_maps(Json(request)).await
}

async fn discover_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Json(request): Json<DiscoverRequest>,
) -> Result<Json<ApiResponse<DiscoveryReport>>, ApiError> {
    handlers.discover_maps(Json(request)).await
}

async fn compact_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
) -> Result<Json<ApiResponse<CompactReport>>, ApiError> {
    handlers.compact_registry().await
}

async fn modify_map_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Path(id): Path<String>,
    Json(request): Json<ModifyMapRequest>,
) -> Result<Json<ApiResponse<MapEntry>>, ApiError> {
    handlers.modify_map(Path(id), Json(request)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    struct TestApp {
        router: Router,
        _dirs: test_helpers::TestDirs,
    }

    async fn test_app() -> TestApp {
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
        let handlers = Arc::new(ApiHandlers::new(registry, installer));
        TestApp {
            router: HttpServer::router(handlers),
            _dirs: dirs,
        }
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let TestApp { router: app, _dirs } = test_app().await;
        let response = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_list_maps_empty() {
        let TestApp { router: app, _dirs } = test_app().await;
        let response = app
            .oneshot(Request::get("/api/maps").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_get_map_not_found_returns_json_error() {
        let TestApp { router: app, _dirs } = test_app().await;
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
        let TestApp { router: app, _dirs } = test_app().await;
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
        let TestApp { router: app, _dirs } = test_app().await;
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
        let TestApp { router: app, _dirs } = test_app().await;
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
        let TestApp { router: app, _dirs } = test_app().await;
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
        let TestApp { router: app, _dirs } = test_app().await;
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
        let TestApp { router: app, _dirs } = test_app().await;
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
