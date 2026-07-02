// SPDX-License-Identifier: GPL-3.0-only
use axum::extract::Path;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info};

use crate::registry::{MapEntry, Registry};
use crate::map_installer::{
    CompactReport, DiscoveryMode, DiscoveryReport, MapInstallationService, WorkshopUpdateReport,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct InstallMapRequest {
    /// HTTP/HTTPS URL for ZIP file download (only used when workshop_id is not provided)
    pub url: Option<String>,
    
    /// Steam Workshop ID (only used when url is not provided)
    pub workshop_id: Option<u64>,
    
    /// Optional map name override
    pub name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateWorkshopRequest {
    pub map_id: Option<u64>,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub check_only: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiscoverRequest {
    #[serde(default)]
    pub mode: DiscoveryMode,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModifyMapRequest {
    pub field: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }
    
    pub fn error(message: String) -> ApiResponse<()> {
        ApiResponse {
            success: false,
            data: None,
            error: Some(message),
        }
    }
}

pub struct ApiHandlers {
    registry: Arc<dyn Registry>,
    installer: Arc<MapInstallationService>,
}

impl ApiHandlers {
    pub fn new(registry: Arc<dyn Registry>, installer: Arc<MapInstallationService>) -> Self {
        Self { registry, installer }
    }
}

impl ApiHandlers {
    pub async fn list_maps(
        &self,
    ) -> Result<Json<ApiResponse<Vec<MapEntry>>>, StatusCode> {
        match self.registry.list_maps().await {
            Ok(maps) => Ok(Json(ApiResponse::success(maps))),
            Err(e) => {
                error!(error = %e, "Failed to list maps");
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }
    
    pub async fn get_map(
        &self,
        Path(id): Path<String>,
    ) -> Result<Json<ApiResponse<MapEntry>>, StatusCode> {
        let map_id = id.parse::<u64>()
            .map_err(|_| {
                error!(id = %id, "Invalid map ID format (expected integer)");
                StatusCode::BAD_REQUEST
            })?;
        
        match self.registry.get_map(map_id).await {
            Ok(Some(map)) => Ok(Json(ApiResponse::success(map))),
            Ok(None) => Err(StatusCode::NOT_FOUND),
            Err(e) => {
                error!(error = %e, "Failed to get map");
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }
    
    pub async fn install_map(
        &self,
        Json(request): Json<InstallMapRequest>,
    ) -> Result<Json<ApiResponse<u64>>, StatusCode> {
        // Validate that exactly one of url or workshop_id is provided
        match (request.url.as_ref(), request.workshop_id) {
            (Some(_), Some(_)) => {
                error!("Both url and workshop_id provided, but only one is allowed");
                return Err(StatusCode::BAD_REQUEST);
            }
            (None, None) => {
                error!("Neither url nor workshop_id provided, one is required");
                return Err(StatusCode::BAD_REQUEST);
            }
            (Some(url), None) => {
                info!(url = %url, "Install map request received with URL");
                
                // Validate URL length
                if url.len() > 2048 {
                    error!("URL too long: {} characters", url.len());
                    return Err(StatusCode::BAD_REQUEST);
                }
                
                // Validate map name length if provided
                if let Some(ref name) = request.name {
                    if name.len() > 255 {
                        error!("Map name too long: {} characters", name.len());
                        return Err(StatusCode::BAD_REQUEST);
                    }
                }
                
                // Install from URL
                match self.installer.install_from_url(url.clone(), request.name).await {
                    Ok(map_entry) => {
                        info!(map_id = %map_entry.id, "Map installed successfully");
                        Ok(Json(ApiResponse::success(map_entry.id)))
                    }
                    Err(e) => {
                        error!(error = %e, "Failed to install map");
                        Err(StatusCode::INTERNAL_SERVER_ERROR)
                    }
                }
            }
            (None, Some(workshop_id)) => {
                info!(workshop_id, "Install map request received with workshop ID");
                
                // Validate map name length if provided
                if let Some(ref name) = request.name {
                    if name.len() > 255 {
                        error!("Map name too long: {} characters", name.len());
                        return Err(StatusCode::BAD_REQUEST);
                    }
                }
                
                // Install from workshop ID
                match self.installer.install_from_workshop_id(workshop_id, request.name).await {
                    Ok(map_entry) => {
                        info!(map_id = %map_entry.id, "Map installed successfully");
                        Ok(Json(ApiResponse::success(map_entry.id)))
                    }
                    Err(e) => {
                        error!(error = %e, "Failed to install map");
                        Err(StatusCode::INTERNAL_SERVER_ERROR)
                    }
                }
            }
        }
    }
    
    pub async fn uninstall_map(
        &self,
        Path(id): Path<String>,
    ) -> Result<Json<ApiResponse<()>>, StatusCode> {
        let map_id = id.parse::<u64>()
            .map_err(|_| {
                error!(id = %id, "Invalid map ID format (expected integer)");
                StatusCode::BAD_REQUEST
            })?;
        
        match self.installer.uninstall_map(map_id).await {
            Ok(()) => {
                info!(map_id = map_id, "Map uninstalled");
                Ok(Json(ApiResponse::success(())))
            }
            Err(e) => {
                error!(error = %e, "Failed to uninstall map");
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }

    pub async fn update_workshop_maps(
        &self,
        Json(request): Json<UpdateWorkshopRequest>,
    ) -> Result<Json<ApiResponse<WorkshopUpdateReport>>, StatusCode> {
        info!(
            map_id = ?request.map_id,
            force = request.force,
            check_only = request.check_only,
            "Workshop update request received"
        );

        match self
            .installer
            .update_workshop_maps(request.map_id, request.force, request.check_only)
            .await
        {
            Ok(report) => Ok(Json(ApiResponse::success(report))),
            Err(e) => {
                let message = e.to_string();
                if message.contains("not found") {
                    error!(error = %message, "Workshop update target not found");
                    return Err(StatusCode::NOT_FOUND);
                }
                error!(error = %message, "Workshop update failed");
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }

    pub async fn discover_maps(
        &self,
        Json(request): Json<DiscoverRequest>,
    ) -> Result<Json<ApiResponse<DiscoveryReport>>, StatusCode> {
        info!(mode = ?request.mode, "Discover maps request received");

        match self.installer.discover_maps(request.mode).await {
            Ok(report) => Ok(Json(ApiResponse::success(report))),
            Err(e) => {
                error!(error = %e, "Discovery failed");
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }

    pub async fn compact_registry(
        &self,
    ) -> Result<Json<ApiResponse<CompactReport>>, StatusCode> {
        info!("Compact registry request received");

        match self.installer.compact_registry().await {
            Ok(report) => Ok(Json(ApiResponse::success(report))),
            Err(e) => {
                error!(error = %e, "Compact failed");
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }

    pub async fn modify_map(
        &self,
        Path(id): Path<String>,
        Json(request): Json<ModifyMapRequest>,
    ) -> Result<Json<ApiResponse<MapEntry>>, StatusCode> {
        let map_id = id.parse::<u64>().map_err(|_| {
            error!(id = %id, "Invalid map ID format (expected integer)");
            StatusCode::BAD_REQUEST
        })?;

        if request.field.is_empty() {
            error!("Modify request missing field name");
            return Err(StatusCode::BAD_REQUEST);
        }

        if request.value.len() > 2048 {
            error!("Modify value too long: {} characters", request.value.len());
            return Err(StatusCode::BAD_REQUEST);
        }

        info!(map_id, field = %request.field, "Modify map request received");

        match self
            .installer
            .modify_map_field(map_id, &request.field, &request.value)
            .await
        {
            Ok(map) => Ok(Json(ApiResponse::success(map))),
            Err(e) => {
                let message = e.to_string();
                if message.contains("not found") {
                    error!(map_id, error = %message, "Map not found for modify");
                    return Err(StatusCode::NOT_FOUND);
                }
                if message.contains("Unknown or read-only field")
                    || message.contains("Invalid source_kind")
                    || message.contains("Invalid workshop_id")
                {
                    error!(map_id, error = %message, "Invalid modify field or value");
                    return Err(StatusCode::BAD_REQUEST);
                }
                error!(map_id, error = %message, "Failed to modify map");
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::models::SourceKind;
    use crate::registry::traits::Registry;
    use crate::test_helpers::{self, TestDirs};
    use axum::extract::Path;
    use std::sync::Arc;

    async fn setup_handlers() -> (ApiHandlers, Arc<dyn Registry>, TestDirs) {
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
        (
            ApiHandlers::new(Arc::clone(&registry), installer),
            registry,
            dirs,
        )
    }

    fn sample_map() -> MapEntry {
        MapEntry {
            id: 0,
            name: "Test Map".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            source_kind: SourceKind::Other,
            workshop_id: None,
            installed_path: "test_map.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: None,
            checksum: None,
            checksum_kind: None,
        }
    }

    #[tokio::test]
    async fn test_modify_map_success() {
        let (handlers, registry, _dirs) = setup_handlers().await;
        let id = registry.add_map(sample_map()).await.unwrap();

        let response = handlers
            .modify_map(
                Path(id.to_string()),
                Json(ModifyMapRequest {
                    field: "name".to_string(),
                    value: "Renamed Map".to_string(),
                }),
            )
            .await
            .unwrap();

        assert!(response.0.success);
        assert_eq!(response.0.data.as_ref().unwrap().name, "Renamed Map");
    }

    #[tokio::test]
    async fn test_modify_map_not_found() {
        let (handlers, _registry, _dirs) = setup_handlers().await;

        let result = handlers
            .modify_map(
                Path("99999".to_string()),
                Json(ModifyMapRequest {
                    field: "name".to_string(),
                    value: "Renamed Map".to_string(),
                }),
            )
            .await;

        assert_eq!(result.unwrap_err(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_modify_map_unknown_field() {
        let (handlers, registry, _dirs) = setup_handlers().await;
        let id = registry.add_map(sample_map()).await.unwrap();

        let result = handlers
            .modify_map(
                Path(id.to_string()),
                Json(ModifyMapRequest {
                    field: "installed_path".to_string(),
                    value: "other.vpk".to_string(),
                }),
            )
            .await;

        assert_eq!(result.unwrap_err(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_discover_maps_empty_addons() {
        let (handlers, _registry, _dirs) = setup_handlers().await;

        let response = handlers
            .discover_maps(Json(DiscoverRequest {
                mode: DiscoveryMode::Add,
            }))
            .await
            .unwrap();

        assert!(response.0.success);
        let report = response.0.data.unwrap();
        assert!(report.added.is_empty());
        assert!(report.updated.is_empty());
    }

    #[tokio::test]
    async fn test_compact_registry_empty() {
        let (handlers, _registry, _dirs) = setup_handlers().await;

        let response = handlers.compact_registry().await.unwrap();

        assert!(response.0.success);
        let report = response.0.data.unwrap();
        assert!(report.removed.is_empty());
        assert!(report.kept.is_empty());
    }

    #[tokio::test]
    async fn test_update_workshop_maps_empty_registry() {
        let (handlers, _registry, _dirs) = setup_handlers().await;

        let response = handlers
            .update_workshop_maps(Json(UpdateWorkshopRequest {
                map_id: None,
                force: false,
                check_only: true,
            }))
            .await
            .unwrap();

        assert!(response.0.success);
        let report = response.0.data.unwrap();
        assert!(report.available.is_empty());
        assert!(report.updated.is_empty());
    }

    #[tokio::test]
    async fn test_update_workshop_maps_map_not_found() {
        let (handlers, _registry, _dirs) = setup_handlers().await;

        let result = handlers
            .update_workshop_maps(Json(UpdateWorkshopRequest {
                map_id: Some(99999),
                force: false,
                check_only: true,
            }))
            .await;

        assert_eq!(result.unwrap_err(), StatusCode::NOT_FOUND);
    }
}
