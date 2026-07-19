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
        let in_progress = self.installer.active_updates().list();
        let active_ids: std::collections::HashSet<u64> =
            in_progress.iter().map(|u| u.map_id).collect();
        let available = self
            .installer
            .pending_updates()
            .list()
            .into_iter()
            .filter(|u| !active_ids.contains(&u.map_id))
            .collect();
        Ok(ok_json(MapUpdatesStatus {
            available,
            in_progress,
        }))
    }

    pub async fn check_available_updates(
        &self,
    ) -> Result<Json<ApiResponse<MapUpdatesStatus>>, ApiError> {
        info!("Manual bulk update check request received");
        let _check_guard = self.installer.try_lock_updates_check().ok_or_else(|| {
            ApiError::conflict("A map update check is already in progress")
        })?;

        let current_config = crate::config::read_config(&self.config);
        let pending = self.installer.pending_updates();

        let workshop_enabled = current_config.workshop_update_check_enabled;
        let l4d2_enabled = current_config.l4d2center_update_check_enabled;
        let mut enabled_attempts = 0usize;
        let mut enabled_successes = 0usize;

        // 1. Check Workshop
        if workshop_enabled {
            enabled_attempts += 1;
            match self
                .installer
                .update_workshop_maps(None, false, true)
                .await
            {
                Ok(report) => {
                    if !report.failed.is_empty() {
                        tracing::warn!(
                            failed = report.failed.len(),
                            "Workshop check reported per-map failures"
                        );
                    }
                    let available: Vec<_> = report
                        .available
                        .iter()
                        .map(|item| crate::map_installer::AvailableMapUpdate {
                            name: item.map.name.clone(),
                            map_id: item.map.id,
                            source_kind: crate::registry::SourceKind::Workshop,
                            workshop_id: Some(item.workshop_id),
                        })
                        .collect();
                    let exclude = self.installer.active_updates().active_ids();
                    pending.replace_for_source_excluding(
                        crate::registry::SourceKind::Workshop,
                        available,
                        &exclude,
                    );
                    enabled_successes += 1;
                }
                Err(e) => {
                    tracing::error!(error = %e, "Manual workshop update check failed");
                    pending.replace_for_source(crate::registry::SourceKind::Workshop, vec![]);
                }
            }
        } else {
            pending.replace_for_source(crate::registry::SourceKind::Workshop, vec![]);
        }

        // 2. Check L4D2Center
        if l4d2_enabled {
            enabled_attempts += 1;
            match self
                .installer
                .update_l4d2center_maps(
                    &current_config.l4d2center_index_url,
                    None,
                    None,
                    false,
                    true,
                )
                .await
            {
                Ok(report) => {
                    if !report.failed.is_empty() {
                        tracing::warn!(
                            failed = report.failed.len(),
                            "L4D2Center check reported per-map failures"
                        );
                    }
                    let available: Vec<_> = report
                        .available
                        .iter()
                        .map(|item| crate::map_installer::AvailableMapUpdate {
                            name: item.name.clone(),
                            map_id: item.map_id,
                            source_kind: crate::registry::SourceKind::L4d2Center,
                            workshop_id: None,
                        })
                        .collect();
                    let exclude = self.installer.active_updates().active_ids();
                    pending.replace_for_source_excluding(
                        crate::registry::SourceKind::L4d2Center,
                        available,
                        &exclude,
                    );
                    enabled_successes += 1;
                }
                Err(e) => {
                    tracing::error!(error = %e, "Manual L4D2Center update check failed");
                    pending.replace_for_source(crate::registry::SourceKind::L4d2Center, vec![]);
                }
            }
        } else {
            pending.replace_for_source(crate::registry::SourceKind::L4d2Center, vec![]);
        }

        // If every enabled source failed, surface an error instead of "no updates".
        if enabled_attempts > 0 && enabled_successes == 0 {
            return Err(ApiError::internal(
                "Map update check failed for all enabled sources",
            ));
        }

        // Return updated status
        self.list_available_updates().await
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
