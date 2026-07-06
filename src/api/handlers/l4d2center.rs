// SPDX-License-Identifier: GPL-3.0-only
use axum::Json;
use tracing::info;

use crate::api::error::ApiError;
use crate::api::service_error::classify_l4d2center_error;
use crate::api::types::{InstallL4d2CenterRequest, UpdateL4d2CenterRequest};
use crate::catalog::L4d2CenterCatalogEntry;
use crate::map_installer::L4d2CenterUpdateReport;
use crate::registry::MapEntry;

use super::helpers::{ok_json};
use super::ApiHandlers;

impl ApiHandlers {
    pub async fn list_l4d2center_catalog(
        &self,
    ) -> Result<Json<crate::api::response::ApiResponse<Vec<L4d2CenterCatalogEntry>>>, ApiError>
    {
        match self
            .installer
            .list_l4d2center_catalog(&self.l4d2center_index_url())
            .await
        {
            Ok(catalog) => Ok(ok_json(catalog)),
            Err(error) => Err(classify_l4d2center_error(error)),
        }
    }

    pub async fn install_l4d2center_map(
        &self,
        Json(request): Json<InstallL4d2CenterRequest>,
    ) -> Result<Json<crate::api::response::ApiResponse<MapEntry>>, ApiError> {
        info!(name = %request.name, "Install L4D2Center map request received");
        match self
            .installer
            .install_l4d2center_by_name(&self.l4d2center_index_url(), &request.name)
            .await
        {
            Ok(map_entry) => Ok(ok_json(map_entry)),
            Err(error) => Err(classify_l4d2center_error(error)),
        }
    }

    pub async fn update_l4d2center_maps(
        &self,
        Json(request): Json<UpdateL4d2CenterRequest>,
    ) -> Result<Json<crate::api::response::ApiResponse<L4d2CenterUpdateReport>>, ApiError> {
        match self
            .installer
            .update_l4d2center_maps(
                &self.l4d2center_index_url(),
                request.map_id,
                request.name.as_deref(),
                request.force,
                request.check_only,
            )
            .await
        {
            Ok(report) => Ok(ok_json(report)),
            Err(error) => Err(classify_l4d2center_error(error)),
        }
    }
}
