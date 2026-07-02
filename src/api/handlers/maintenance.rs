// SPDX-License-Identifier: GPL-3.0-only
use axum::Json;
use tracing::info;

use crate::api::error::ApiError;
use crate::api::response::ApiResponse;
use crate::api::service_error::classify_workshop_error;
use crate::api::types::{DiscoverRequest, UpdateWorkshopRequest};
use crate::map_installer::{CompactReport, DiscoveryReport, WorkshopUpdateReport};

use super::helpers::{installer_internal_err, ok_json};
use super::ApiHandlers;

impl ApiHandlers {
    pub async fn update_workshop_maps(
        &self,
        Json(request): Json<UpdateWorkshopRequest>,
    ) -> Result<Json<ApiResponse<WorkshopUpdateReport>>, ApiError> {
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
            Ok(report) => Ok(ok_json(report)),
            Err(e) => Err(classify_workshop_error(e)),
        }
    }

    pub async fn discover_maps(
        &self,
        Json(request): Json<DiscoverRequest>,
    ) -> Result<Json<ApiResponse<DiscoveryReport>>, ApiError> {
        info!(mode = ?request.mode, "Discover maps request received");

        match self.installer.discover_maps(request.mode).await {
            Ok(report) => Ok(ok_json(report)),
            Err(e) => Err(installer_internal_err(e, "Discovery failed")),
        }
    }

    pub async fn compact_registry(
        &self,
    ) -> Result<Json<ApiResponse<CompactReport>>, ApiError> {
        info!("Compact registry request received");

        match self.installer.compact_registry().await {
            Ok(report) => Ok(ok_json(report)),
            Err(e) => Err(installer_internal_err(e, "Compact failed")),
        }
    }
}
