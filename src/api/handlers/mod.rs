// SPDX-License-Identifier: GPL-3.0-only
mod maintenance;
mod maps;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use axum::Json;

use crate::api::error::ApiError;
use crate::api::response::ApiResponse;
use crate::map_installer::MapInstallationService;
use crate::registry::Registry;

pub struct ApiHandlers {
    pub(super) registry: Arc<dyn Registry>,
    pub(super) installer: Arc<MapInstallationService>,
}

impl ApiHandlers {
    pub fn new(registry: Arc<dyn Registry>, installer: Arc<MapInstallationService>) -> Self {
        Self {
            registry,
            installer,
        }
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
        ApiError::internal(e.to_string())
    }

    pub(super) fn installer_internal_err(
        e: impl std::fmt::Display,
        log_msg: &'static str,
    ) -> ApiError {
        tracing::error!(error = %e, "{log_msg}");
        ApiError::internal(e.to_string())
    }
}
