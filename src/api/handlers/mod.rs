// SPDX-License-Identifier: GPL-3.0-only
mod control;
mod game_server;
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
use crate::map_installer::MapInstallationService;
use crate::maps_denylist::Mapsdenylist;
use crate::registry::Registry;
use crate::repl::DaemonCommand;

pub use control::{DaemonStatus, RestartAccepted, RestartRequest};
pub use game_server::GameServerActionResult;

pub struct ApiHandlers {
    pub(super) registry: Arc<dyn Registry>,
    pub(super) installer: Arc<MapInstallationService>,
    pub(super) config: ConfigHandle,
    pub(super) command_tx: Option<tokio::sync::mpsc::UnboundedSender<DaemonCommand>>,
    /// UID used for `systemctl --user` game server control (0 = current user).
    pub(super) game_server_uid: u32,
    /// Systemd user service name (empty = default `l4d2-srcds1.service`).
    pub(super) game_server_service: String,
}

impl ApiHandlers {
    pub fn new(
        registry: Arc<dyn Registry>,
        installer: Arc<MapInstallationService>,
        config: ConfigHandle,
    ) -> Self {
        Self {
            registry,
            installer,
            config,
            command_tx: None,
            game_server_uid: unsafe { libc::getuid() },
            game_server_service: String::new(),
        }
    }

    /// Enables lifecycle endpoints (restart) by giving the handlers the daemon command channel.
    pub fn with_command_sender(
        mut self,
        command_tx: tokio::sync::mpsc::UnboundedSender<DaemonCommand>,
    ) -> Self {
        self.command_tx = Some(command_tx);
        self
    }

    /// Overrides the UID and service name used for game server control.
    pub fn with_game_server_config(mut self, uid: u32, service: String) -> Self {
        self.game_server_uid = uid;
        self.game_server_service = service;
        self
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
