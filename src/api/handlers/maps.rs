// SPDX-License-Identifier: GPL-3.0-only
use axum::extract::Path;
use axum::Json;
use tracing::info;

use crate::api::error::ApiError;
use crate::api::response::ApiResponse;
use crate::api::service_error::{classify_modify_error, classify_uninstall_error};
use crate::api::types::{InstallMapRequest, ModifyMapRequest};
use crate::api::validation::{parse_map_id, validate_install_request, validate_modify_request, InstallSource};
use crate::map_installer::MapUpdatesStatus;
use crate::registry::MapEntry;

use super::helpers::{installer_internal_err, ok_json, registry_internal_err};
use super::ApiHandlers;

impl ApiHandlers {
    pub async fn list_available_updates(
        &self,
    ) -> Result<Json<ApiResponse<MapUpdatesStatus>>, ApiError> {
        Ok(ok_json(MapUpdatesStatus {
            available: self.installer.pending_updates().list(),
            in_progress: self.installer.active_updates().list(),
        }))
    }

    pub async fn list_maps(
        &self,
    ) -> Result<Json<ApiResponse<Vec<MapEntry>>>, ApiError> {
        match self.registry.list_maps().await {
            Ok(maps) => Ok(ok_json(self.denylist().filter_visible(maps))),
            Err(e) => Err(registry_internal_err(e, "Failed to list maps")),
        }
    }

    pub async fn get_map(
        &self,
        Path(id): Path<String>,
    ) -> Result<Json<ApiResponse<MapEntry>>, ApiError> {
        let map_id = parse_map_id(&id)?;

        match self.registry.get_map(map_id).await {
            Ok(Some(map)) => {
                if self.denylist().is_hidden(&map) {
                    return Err(ApiError::not_found(format!("Map #{map_id} not found")));
                }
                Ok(ok_json(map))
            }
            Ok(None) => Err(ApiError::not_found(format!("Map #{map_id} not found"))),
            Err(e) => Err(registry_internal_err(e, "Failed to get map")),
        }
    }

    pub async fn install_map(
        &self,
        Json(request): Json<InstallMapRequest>,
    ) -> Result<Json<ApiResponse<u64>>, ApiError> {
        let source = validate_install_request(&request)?;

        match source {
            InstallSource::Url(url) => {
                info!(url = %url, "Install map request received with URL");
                match self
                    .installer
                    .install_from_url(url, request.name)
                    .await
                {
                    Ok(map_entry) => {
                        info!(map_id = %map_entry.id, "Map installed successfully");
                        Ok(ok_json(map_entry.id))
                    }
                    Err(e) => Err(installer_internal_err(e, "Failed to install map")),
                }
            }
            InstallSource::Workshop(workshop_id) => {
                info!(workshop_id, "Install map request received with workshop ID");
                match self
                    .installer
                    .install_from_workshop_id(workshop_id, request.name)
                    .await
                {
                    Ok(map_entry) => {
                        info!(map_id = %map_entry.id, "Map installed successfully");
                        Ok(ok_json(map_entry.id))
                    }
                    Err(e) => Err(installer_internal_err(e, "Failed to install map")),
                }
            }
        }
    }

    pub async fn uninstall_map(
        &self,
        Path(id): Path<String>,
    ) -> Result<Json<ApiResponse<()>>, ApiError> {
        let map_id = parse_map_id(&id)?;

        match self.installer.uninstall_map(map_id).await {
            Ok(()) => {
                info!(map_id = map_id, "Map uninstalled");
                Ok(ok_json(()))
            }
            Err(e) => Err(classify_uninstall_error(e)),
        }
    }

    pub async fn modify_map(
        &self,
        Path(id): Path<String>,
        Json(request): Json<ModifyMapRequest>,
    ) -> Result<Json<ApiResponse<MapEntry>>, ApiError> {
        let map_id = parse_map_id(&id)?;
        validate_modify_request(&request)?;

        if let Ok(Some(map)) = self.registry.get_map(map_id).await
            && self.denylist().is_hidden(&map)
        {
            return Err(ApiError::not_found(format!("Map #{map_id} not found")));
        }

        info!(map_id, field = %request.field, "Modify map request received");

        match self
            .installer
            .modify_map_field(map_id, &request.field, &request.value)
            .await
        {
            Ok(map) => Ok(ok_json(map)),
            Err(e) => Err(classify_modify_error(e)),
        }
    }
}
