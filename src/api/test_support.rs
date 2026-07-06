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
use crate::config::{init_handle, Config};
#[cfg(test)]
use crate::map_installer::MapInstallationService;
#[cfg(test)]
use crate::registry::traits::Registry;
#[cfg(test)]
use crate::test_helpers::{self, TestDirs};

#[cfg(test)]
pub async fn setup_api_fixture() -> (Arc<ApiHandlers>, Arc<dyn Registry>, TestDirs) {
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
    let mut config = Config::default();
    config.l4d2center_index_url = "https://l4d2center.com/maps/servers/index.json".to_string();
    let config_handle = init_handle(config);
    (
        Arc::new(ApiHandlers::new(
            Arc::clone(&registry),
            installer,
            config_handle,
        )),
        registry,
        dirs,
    )
}

#[cfg(test)]
pub async fn setup_api_router() -> (Router, TestDirs) {
    let (handlers, _registry, dirs) = setup_api_fixture().await;
    (HttpServer::router(handlers), dirs)
}
