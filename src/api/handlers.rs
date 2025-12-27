// SPDX-License-Identifier: GPL-3.0-only
use axum::extract::Path;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info};
use uuid::Uuid;

use crate::registry::{MapEntry, Registry};

#[derive(Debug, Serialize, Deserialize)]
pub struct InstallMapRequest {
    pub url: String,
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
}

impl ApiHandlers {
    pub fn new(registry: Arc<dyn Registry>) -> Self {
        Self { registry }
    }
}

impl ApiHandlers {
    pub async fn health() -> Json<ApiResponse<&'static str>> {
        Json(ApiResponse::success("ok"))
    }
    
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
        match self.registry.get_map(&id).await {
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
    ) -> Result<Json<ApiResponse<String>>, StatusCode> {
        // This is a stub - actual installation would require downloader and extractor
        // The actual implementation would be in the main service loop
        let map_id = Uuid::new_v4().to_string();
        info!(map_id = %map_id, url = %request.url, "Install map request received");
        
        Ok(Json(ApiResponse::success(map_id)))
    }
    
    pub async fn uninstall_map(
        &self,
        Path(id): Path<String>,
    ) -> Result<Json<ApiResponse<()>>, StatusCode> {
        match self.registry.remove_map(&id).await {
            Ok(()) => {
                info!(map_id = %id, "Map uninstalled");
                Ok(Json(ApiResponse::success(())))
            }
            Err(e) => {
                error!(error = %e, "Failed to uninstall map");
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }
}

