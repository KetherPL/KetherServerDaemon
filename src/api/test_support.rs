// SPDX-License-Identifier: GPL-3.0-only
#[cfg(test)]
use std::sync::Arc;

#[cfg(test)]
use axum::Router;

#[cfg(test)]
use crate::api::handlers::ApiHandlers;
#[cfg(test)]
use crate::api::http::HttpServer;
#[cfg(test)]
use crate::config::{init_handle, Config, ConfigHandle};
#[cfg(test)]
use crate::map_installer::{MapInstallationService, PendingUpdatesState};
#[cfg(test)]
use crate::registry::traits::Registry;
#[cfg(test)]
use crate::test_helpers::{self, TestDirs};

#[cfg(test)]
pub async fn setup_api_fixture() -> (Arc<ApiHandlers>, Arc<dyn Registry>, TestDirs) {
    let (handlers, registry, dirs, _config) =
        setup_api_fixture_with_config(Config::default()).await;
    (handlers, registry, dirs)
}

#[cfg(test)]
async fn setup_api_fixture_with_config(
    mut config: Config,
) -> (Arc<ApiHandlers>, Arc<dyn Registry>, TestDirs, ConfigHandle) {
    let (registry, dirs) = test_helpers::setup_test_dirs().await.unwrap();
    let paths = dirs.service_paths();
    let installer = Arc::new(
        MapInstallationService::new(
            Arc::clone(&registry),
            paths.addons_dir,
            paths.download_dir,
            100 * 1024 * 1024,
            1024 * 1024 * 1024,
            10000,
        )
        .await
        .unwrap(),
    );
    config.l4d2center_index_url = "https://l4d2center.com/maps/servers/index.json".to_string();
    let config_handle = init_handle(config);
    let pending_updates = PendingUpdatesState::new();
    (
        Arc::new(ApiHandlers::new(
            Arc::clone(&registry),
            installer,
            config_handle.clone(),
            pending_updates,
        )),
        registry,
        dirs,
        config_handle,
    )
}

#[cfg(test)]
pub async fn setup_api_router() -> (Router, TestDirs) {
    let (handlers, _registry, dirs) = setup_api_fixture().await;
    (HttpServer::router(handlers), dirs)
}

#[cfg(test)]
pub async fn setup_api_router_with_config(config: Config) -> (Router, ConfigHandle, TestDirs) {
    let (handlers, _registry, dirs, config_handle) =
        setup_api_fixture_with_config(config).await;
    (HttpServer::router(handlers), config_handle, dirs)
}
