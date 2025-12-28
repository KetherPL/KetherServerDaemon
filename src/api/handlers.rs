// SPDX-License-Identifier: GPL-3.0-only
use axum::extract::Path;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info};

use crate::registry::{MapEntry, Registry};
use crate::map_installer::MapInstallationService;

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
}

