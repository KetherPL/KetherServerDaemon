// SPDX-License-Identifier: GPL-3.0-only
//! Daemon lifecycle endpoints: status and restart.

use std::time::Duration;

use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

use super::ApiHandlers;
use crate::api::error::ApiError;
use crate::api::response::ApiResponse;
use crate::control;
use crate::repl::DaemonCommand;

/// Delay between answering a restart request and starting the shutdown, so the HTTP
/// response reaches the caller first.
const RESTART_RESPONSE_GRACE: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub version: String,
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub uptime_secs: u64,
    /// What map work is currently running, if any (restarting would wait for / interrupt it).
    pub busy: Option<String>,
    /// Whether this daemon instance accepts restart requests over HTTP.
    pub restart_supported: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RestartRequest {
    /// Restart even if map work is running (it will be interrupted).
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartAccepted {
    /// When the restart was scheduled; a status `started_at` later than this means the
    /// new instance is up.
    pub requested_at: DateTime<Utc>,
    pub forced: bool,
}

impl ApiHandlers {
    pub fn status(&self) -> Json<ApiResponse<DaemonStatus>> {
        Json(ApiResponse::success(DaemonStatus {
            version: env!("CARGO_PKG_VERSION").to_string(),
            pid: std::process::id(),
            started_at: control::started_at(),
            uptime_secs: control::uptime_secs(),
            busy: self.installer.busy_reason().map(str::to_string),
            restart_supported: self.command_tx.is_some(),
        }))
    }

    pub fn restart(
        &self,
        request: RestartRequest,
    ) -> Result<Json<ApiResponse<RestartAccepted>>, ApiError> {
        let Some(command_tx) = self.command_tx.clone() else {
            return Err(ApiError::internal(
                "Restart is not available in this daemon instance",
            ));
        };
        if !request.force
            && let Some(reason) = self.installer.busy_reason()
        {
            return Err(ApiError::conflict(format!(
                "Daemon is busy ({reason}); retry later or force the restart"
            )));
        }

        info!(force = request.force, "Restart requested over HTTP");
        tokio::spawn(async move {
            tokio::time::sleep(RESTART_RESPONSE_GRACE).await;
            let _ = command_tx.send(DaemonCommand::Restart);
        });
        Ok(Json(ApiResponse::success(RestartAccepted {
            requested_at: Utc::now(),
            forced: request.force,
        })))
    }
}
