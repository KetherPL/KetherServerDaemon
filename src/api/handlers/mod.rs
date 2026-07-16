// SPDX-License-Identifier: GPL-3.0-only
mod l4d2center;
mod maintenance;
mod maps;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use axum::Json;

use crate::api::error::ApiError;
use crate::api::response::ApiResponse;
use crate::config::{read_config, ConfigHandle};
use crate::map_installer::{MapInstallationService, PendingUpdatesState};
use crate::maps_denylist::Mapsdenylist;
use crate::registry::Registry;

pub struct ApiHandlers {
    pub(super) registry: Arc<dyn Registry>,
    pub(super) installer: Arc<MapInstallationService>,
    pub(super) config: ConfigHandle,
    pub(super) pending_updates: PendingUpdatesState,
}

impl ApiHandlers {
    pub fn new(
        registry: Arc<dyn Registry>,
        installer: Arc<MapInstallationService>,
        config: ConfigHandle,
        pending_updates: PendingUpdatesState,
    ) -> Self {
        Self {
            registry,
            installer,
            config,
            pending_updates,
        }
    }

    pub(super) fn denylist(&self) -> Mapsdenylist {
        Mapsdenylist::from_config(&read_config(&self.config))
    }

    pub(super) fn l4d2center_index_url(&self) -> String {
        read_config(&self.config).l4d2center_index_url.clone()
    }
}

pub(super) mod helpers {
    use super::*;

    pub(super) fn ok_json<T>(data: T) -> Json<ApiResponse<T>> {
        Json(ApiResponse::success(data))
    }

    pub(super) fn registry_internal_err(
        e: impl std::fmt::Display,
        log_msg: &'static str,
    ) -> ApiError {
        tracing::error!(error = %e, "{log_msg}");
        ApiError::internal("Internal server error")
    }

    pub(super) fn installer_internal_err(
        e: impl std::fmt::Display,
        log_msg: &'static str,
    ) -> ApiError {
        tracing::error!(error = %e, "{log_msg}");
        ApiError::internal("Internal server error")
    }
}
