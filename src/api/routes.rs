// SPDX-License-Identifier: GPL-3.0-only
use axum::{
    extract::Path,
    Json, Router,
};
use std::sync::Arc;

use crate::api::error::ApiError;
use crate::api::handlers::ApiHandlers;
use crate::api::response::ApiResponse;
use crate::api::auth::require_api_key;
use crate::api::types::{
    DiscoverRequest, InstallL4d2CenterRequest, InstallMapRequest, ModifyMapRequest,
    UpdateL4d2CenterRequest, UpdateWorkshopRequest,
};
use crate::catalog::L4d2CenterCatalogEntry;
use crate::map_installer::{
    AvailableMapUpdate, CompactReport, DiscoveryReport, L4d2CenterUpdateReport, WorkshopUpdateReport,
};
use crate::registry::MapEntry;

pub async fn health_handler() -> Json<ApiResponse<&'static str>> {
    Json(ApiResponse::success("ok"))
}

pub async fn list_maps_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
) -> Result<Json<ApiResponse<Vec<MapEntry>>>, ApiError> {
    handlers.list_maps().await
}

pub async fn list_available_updates_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
) -> Result<Json<ApiResponse<Vec<AvailableMapUpdate>>>, ApiError> {
    handlers.list_available_updates().await
}

pub async fn get_map_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<MapEntry>>, ApiError> {
    handlers.get_map(Path(id)).await
}

pub async fn install_map_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Json(request): Json<InstallMapRequest>,
) -> Result<Json<ApiResponse<u64>>, ApiError> {
    handlers.install_map(Json(request)).await
}

pub async fn uninstall_map_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, ApiError> {
    handlers.uninstall_map(Path(id)).await
}

pub async fn update_workshop_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Json(request): Json<UpdateWorkshopRequest>,
) -> Result<Json<ApiResponse<WorkshopUpdateReport>>, ApiError> {
    handlers.update_workshop_maps(Json(request)).await
}

pub async fn discover_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Json(request): Json<DiscoverRequest>,
) -> Result<Json<ApiResponse<DiscoveryReport>>, ApiError> {
    handlers.discover_maps(Json(request)).await
}

pub async fn compact_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
) -> Result<Json<ApiResponse<CompactReport>>, ApiError> {
    handlers.compact_registry().await
}

pub async fn update_l4d2center_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Json(request): Json<UpdateL4d2CenterRequest>,
) -> Result<Json<ApiResponse<L4d2CenterUpdateReport>>, ApiError> {
    handlers.update_l4d2center_maps(Json(request)).await
}

pub async fn list_l4d2center_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
) -> Result<Json<ApiResponse<Vec<L4d2CenterCatalogEntry>>>, ApiError> {
    handlers.list_l4d2center_catalog().await
}

pub async fn install_l4d2center_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Json(request): Json<InstallL4d2CenterRequest>,
) -> Result<Json<ApiResponse<MapEntry>>, ApiError> {
    handlers.install_l4d2center_map(Json(request)).await
}

pub async fn modify_map_handler(
    axum::extract::State(handlers): axum::extract::State<Arc<ApiHandlers>>,
    Path(id): Path<String>,
    Json(request): Json<ModifyMapRequest>,
) -> Result<Json<ApiResponse<MapEntry>>, ApiError> {
    handlers.modify_map(Path(id), Json(request)).await
}

pub fn routes(handlers: Arc<ApiHandlers>) -> Router {
    use axum::middleware;
    use axum::routing::{get, post};

    let protected = Router::new()
        .route("/api/maps/install", post(install_map_handler))
        .route("/api/maps/uninstall/{id}", post(uninstall_map_handler))
        .route("/api/maps/workshop/update", post(update_workshop_handler))
        .route("/api/maps/l4d2center", get(list_l4d2center_handler))
        .route("/api/maps/l4d2center/install", post(install_l4d2center_handler))
        .route("/api/maps/l4d2center/update", post(update_l4d2center_handler))
        .route("/api/maps/discover", post(discover_handler))
        .route("/api/maps/compact", post(compact_handler))
        .route("/api/maps/updates/available", get(list_available_updates_handler))
        .route("/api/maps", get(list_maps_handler))
        .route(
            "/api/maps/{id}",
            get(get_map_handler).patch(modify_map_handler),
        )
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&handlers),
            require_api_key,
        ))
        .with_state(Arc::clone(&handlers));

    Router::new()
        .route("/health", get(health_handler))
        .merge(protected)
        .with_state(handlers)
}
