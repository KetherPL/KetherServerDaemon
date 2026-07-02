// SPDX-License-Identifier: GPL-3.0-only
use axum::{
    extract::Path,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;
use crate::api::handlers::{
    ApiHandlers, ApiResponse, DiscoverRequest, InstallMapRequest, ModifyMapRequest,
    UpdateWorkshopRequest,
};
use crate::registry::{MapEntry, Registry};
use crate::map_installer::{CompactReport, DiscoveryReport, MapInstallationService, WorkshopUpdateReport};

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
    
    pub async fn serve(self) -> anyhow::Result<()> {
        let handlers = Arc::new(self.handlers);
        
        let app = Router::new()
            .route("/api/maps", get(list_maps_handler))
            .route("/api/maps/{id}", get(get_map_handler).patch(modify_map_handler))
            .route("/api/maps/install", post(install_map_handler))
            .route("/api/maps/uninstall/{id}", post(uninstall_map_handler))
            .route("/api/maps/workshop/update", post(update_workshop_handler))
            .route("/api/maps/discover", post(discover_handler))
            .route("/api/maps/compact", post(compact_handler))
            .with_state(handlers.clone());
        
        info!(addr = %self.addr, "Starting HTTP server");
        
        let listener = tokio::net::TcpListener::bind(&self.addr).await?;
        axum::serve(listener, app).await?;
        
        Ok(())
    }
}

async fn list_maps_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
) -> Result<Json<ApiResponse<Vec<MapEntry>>>, StatusCode> {
    handlers.list_maps().await
}

async fn get_map_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<MapEntry>>, StatusCode> {
    handlers.get_map(Path(id)).await
}

async fn install_map_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Json(request): Json<InstallMapRequest>,
) -> Result<Json<ApiResponse<u64>>, StatusCode> {
    handlers.install_map(Json(request)).await
}

async fn uninstall_map_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, StatusCode> {
    handlers.uninstall_map(Path(id)).await
}

async fn update_workshop_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Json(request): Json<UpdateWorkshopRequest>,
) -> Result<Json<ApiResponse<WorkshopUpdateReport>>, StatusCode> {
    handlers.update_workshop_maps(Json(request)).await
}

async fn discover_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Json(request): Json<DiscoverRequest>,
) -> Result<Json<ApiResponse<DiscoveryReport>>, StatusCode> {
    handlers.discover_maps(Json(request)).await
}

async fn compact_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
) -> Result<Json<ApiResponse<CompactReport>>, StatusCode> {
    handlers.compact_registry().await
}

async fn modify_map_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Path(id): Path<String>,
    Json(request): Json<ModifyMapRequest>,
) -> Result<Json<ApiResponse<MapEntry>>, StatusCode> {
    handlers.modify_map(Path(id), Json(request)).await
}

