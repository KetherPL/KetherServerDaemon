// SPDX-License-Identifier: GPL-3.0-only
//! Game server (L4D2) lifecycle endpoints: status, start, stop, restart.

use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::info;

use super::ApiHandlers;
use crate::api::error::ApiError;
use crate::api::response::ApiResponse;
use crate::game_server::{
    game_server_action, game_server_status, GameServerStatus, ServiceAction, DEFAULT_L4D2_SERVICE,
};

/// Returned by start/stop/restart; mirrors the service status right after the action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameServerActionResult {
    pub action: String,
    pub status: GameServerStatus,
}

fn uid_from_handlers(handlers: &ApiHandlers) -> u32 {
    handlers.game_server_uid
}

fn service_from_handlers(handlers: &ApiHandlers) -> String {
    if handlers.game_server_service.is_empty() {
        DEFAULT_L4D2_SERVICE.to_string()
    } else {
        handlers.game_server_service.clone()
    }
}

impl ApiHandlers {
    pub fn game_server_status_handler(&self) -> Json<ApiResponse<GameServerStatus>> {
        let uid = uid_from_handlers(self);
        let service = service_from_handlers(self);
        let status = game_server_status(uid, &service);
        Json(ApiResponse::success(status))
    }

    pub fn game_server_action_handler(
        &self,
        action: ServiceAction,
    ) -> Result<Json<ApiResponse<GameServerActionResult>>, ApiError> {
        let uid = uid_from_handlers(self);
        let service = service_from_handlers(self);
        info!(action = %action, service = %service, uid, "Game server action requested via API");
        game_server_action(uid, &service, action)
            .map_err(|e| ApiError::internal(format!("Game server control failed: {e}")))?;
        let status = game_server_status(uid, &service);
        Ok(Json(ApiResponse::success(GameServerActionResult {
            action: action.to_string(),
            status,
        })))
    }
}
