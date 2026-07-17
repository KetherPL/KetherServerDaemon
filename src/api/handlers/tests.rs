// SPDX-License-Identifier: GPL-3.0-only
use axum::extract::Path;
use axum::Json;

use crate::api::types::{DiscoverRequest, InstallMapRequest, ModifyMapRequest, UpdateWorkshopRequest};
use crate::map_installer::DiscoveryMode;
use crate::registry::models::SourceKind;
use crate::registry::MapEntry;

use crate::api::test_support::setup_api_fixture;

fn sample_map() -> MapEntry {
    MapEntry {
        id: 0,
        name: "Test Map".to_string(),
        source_url: "https://example.com/map.zip".to_string(),
        source_kind: SourceKind::Other,
        workshop_id: None,
        installed_path: "test_map.vpk".to_string(),
        installed_at: chrono::Utc::now(),
        workshop_updated_at: None,
        version: None,
        checksum: None,
        checksum_kind: None,
    }
}

#[tokio::test]
async fn test_modify_map_success() {
    let (handlers, registry, _dirs) = setup_api_fixture().await;
    let id = registry.add_map(sample_map()).await.unwrap();

    let response = handlers
        .modify_map(
            Path(id.to_string()),
            Json(ModifyMapRequest {
                field: "name".to_string(),
                value: "Renamed Map".to_string(),
            }),
        )
        .await
        .unwrap();

    assert!(response.0.success);
    assert_eq!(response.0.data.as_ref().unwrap().name, "Renamed Map");
}

#[tokio::test]
async fn test_modify_map_not_found() {
    let (handlers, _registry, _dirs) = setup_api_fixture().await;

    let result = handlers
        .modify_map(
            Path("99999".to_string()),
            Json(ModifyMapRequest {
                field: "name".to_string(),
                value: "Renamed Map".to_string(),
            }),
        )
        .await;

    assert_eq!(
        result.unwrap_err().status_code(),
        axum::http::StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn test_modify_map_unknown_field() {
    let (handlers, registry, _dirs) = setup_api_fixture().await;
    let id = registry.add_map(sample_map()).await.unwrap();

    let result = handlers
        .modify_map(
            Path(id.to_string()),
            Json(ModifyMapRequest {
                field: "checksum".to_string(),
                value: "abc123".to_string(),
            }),
        )
        .await;

    assert_eq!(
        result.unwrap_err().status_code(),
        axum::http::StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn test_modify_map_installed_path_success() {
    use crate::test_helpers;

    let (handlers, registry, dirs) = setup_api_fixture().await;
    let addons = dirs.addons_path();
    test_helpers::write_minimal_test_vpk(&addons.join("test_map.vpk"), "Test Map").unwrap();
    let id = registry.add_map(sample_map()).await.unwrap();

    let response = handlers
        .modify_map(
            Path(id.to_string()),
            Json(ModifyMapRequest {
                field: "installed_path".to_string(),
                value: "renamed.vpk".to_string(),
            }),
        )
        .await
        .unwrap();

    assert!(response.0.success);
    assert_eq!(
        response.0.data.as_ref().unwrap().installed_path,
        "renamed.vpk"
    );
    assert!(!addons.join("test_map.vpk").exists());
    assert!(addons.join("renamed.vpk").exists());
}

#[tokio::test]
async fn test_discover_maps_empty_addons() {
    let (handlers, _registry, _dirs) = setup_api_fixture().await;

    let response = handlers
        .discover_maps(Json(DiscoverRequest {
            mode: DiscoveryMode::Add,
        }))
        .await
        .unwrap();

    assert!(response.0.success);
    let report = response.0.data.unwrap();
    assert!(report.added.is_empty());
    assert!(report.updated.is_empty());
}

#[tokio::test]
async fn test_compact_registry_empty() {
    let (handlers, _registry, _dirs) = setup_api_fixture().await;

    let response = handlers.compact_registry().await.unwrap();

    assert!(response.0.success);
    let report = response.0.data.unwrap();
    assert!(report.removed.is_empty());
    assert!(report.kept.is_empty());
}

#[tokio::test]
async fn test_update_workshop_maps_empty_registry() {
    let (handlers, _registry, _dirs) = setup_api_fixture().await;

    let response = handlers
        .update_workshop_maps(Json(UpdateWorkshopRequest {
            map_id: None,
            force: false,
            check_only: true,
        }))
        .await
        .unwrap();

    assert!(response.0.success);
    let report = response.0.data.unwrap();
    assert!(report.available.is_empty());
    assert!(report.updated.is_empty());
}

#[tokio::test]
async fn test_update_workshop_maps_map_not_found() {
    let (handlers, _registry, _dirs) = setup_api_fixture().await;

    let result = handlers
        .update_workshop_maps(Json(UpdateWorkshopRequest {
            map_id: Some(99999),
            force: false,
            check_only: true,
        }))
        .await;

    assert_eq!(
        result.unwrap_err().status_code(),
        axum::http::StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn test_list_maps_returns_entries() {
    let (handlers, registry, _dirs) = setup_api_fixture().await;
    let id = registry.add_map(sample_map()).await.unwrap();

    let response = handlers.list_maps().await.unwrap();
    assert!(response.0.success);
    let maps = response.0.data.unwrap();
    assert_eq!(maps.len(), 1);
    assert_eq!(maps[0].id, id);
}

#[tokio::test]
async fn test_list_maps_excludes_denylisted() {
    use std::sync::Arc;

    use crate::api::handlers::ApiHandlers;
    use crate::config::init_handle;
    use crate::map_installer::MapInstallationService;
    use crate::registry::models::SourceKind;

    let (registry, dirs) = crate::test_helpers::setup_test_dirs().await.unwrap();
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

    let visible_id = registry
        .add_map(MapEntry {
            id: 0,
            name: "Visible".to_string(),
            source_url: String::new(),
            source_kind: SourceKind::Workshop,
            workshop_id: Some(111),
            installed_path: "visible.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: None,
            checksum: None,
            checksum_kind: None,
        })
        .await
        .unwrap();

    let hidden_workshop_id = registry
        .add_map(MapEntry {
            id: 0,
            name: "Hidden Workshop".to_string(),
            source_url: String::new(),
            source_kind: SourceKind::Workshop,
            workshop_id: Some(381419931),
            installed_path: "hidden_workshop.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: None,
            checksum: None,
            checksum_kind: None,
        })
        .await
        .unwrap();

    let hidden_internal_id = registry
        .add_map(MapEntry {
            id: 0,
            name: "Hidden Internal".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            source_kind: SourceKind::Other,
            workshop_id: None,
            installed_path: "hidden_internal.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: None,
            checksum: None,
            checksum_kind: None,
        })
        .await
        .unwrap();

    let mut config = crate::config::Config::default();
    config.hidden_workshop_ids = vec![381419931];
    config.hidden_map_ids = vec![hidden_internal_id];
    let config_handle = init_handle(config);

    let handlers = Arc::new(ApiHandlers::new(
        Arc::clone(&registry),
        installer,
        config_handle,
    ));

    let response = handlers.list_maps().await.unwrap();
    let maps = response.0.data.unwrap();
    assert_eq!(maps.len(), 1);
    assert_eq!(maps[0].id, visible_id);
    assert_eq!(maps[0].workshop_id, Some(111));

    let hidden_workshop = registry.get_map(hidden_workshop_id).await.unwrap();
    assert!(hidden_workshop.is_some());
}

#[tokio::test]
async fn test_list_maps_reflects_hot_reloaded_denylist() {
    use std::sync::Arc;

    use crate::api::handlers::ApiHandlers;
    use crate::config::init_handle;
    use crate::config_watch::apply_reload;
    use crate::map_installer::MapInstallationService;
    use crate::registry::models::SourceKind;

    let (registry, dirs) = crate::test_helpers::setup_test_dirs().await.unwrap();
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

    registry
        .add_map(MapEntry {
            id: 0,
            name: "Workshop Map".to_string(),
            source_url: String::new(),
            source_kind: SourceKind::Workshop,
            workshop_id: Some(381419931),
            installed_path: "workshop.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: None,
            checksum: None,
            checksum_kind: None,
        })
        .await
        .unwrap();

    let tmp = tempfile::tempdir().expect("tempdir");
    let config_path = tmp.path().join(crate::config::CONF_FILE_NAME);
    let initial_toml = r#"
l4d2_server_dir = "/home/steam/l4d2"
registry_path = "registry.json"
backend_api_url = "http://127.0.0.1:3001/api"
local_api_bind = "127.0.0.1:8080"
sync_interval_secs = 300
log_level = "info"
hidden_workshop_ids = [381419931]
hidden_map_ids = []
"#;
    std::fs::write(&config_path, initial_toml).expect("write config");

    let config = crate::config::Config::load_from(&config_path).expect("load config");
    let config_handle = init_handle(config);
    let handlers = Arc::new(ApiHandlers::new(
        Arc::clone(&registry),
        installer,
        config_handle.clone(),
    ));

    let hidden = handlers.list_maps().await.unwrap();
    assert!(hidden.0.data.unwrap().is_empty());

    std::fs::write(
        &config_path,
        r#"
l4d2_server_dir = "/home/steam/l4d2"
registry_path = "registry.json"
backend_api_url = "http://127.0.0.1:3001/api"
local_api_bind = "127.0.0.1:8080"
sync_interval_secs = 300
log_level = "info"
hidden_workshop_ids = []
hidden_map_ids = []
"#,
    )
    .expect("write updated config");

    let change = apply_reload(&config_handle, &config_path).expect("reload");
    assert!(change.live_applied.contains(&"hidden_workshop_ids"));

    let visible = handlers.list_maps().await.unwrap();
    assert_eq!(visible.0.data.unwrap().len(), 1);
}

#[tokio::test]
async fn test_get_map_success() {
    let (handlers, registry, _dirs) = setup_api_fixture().await;
    let id = registry.add_map(sample_map()).await.unwrap();

    let response = handlers.get_map(Path(id.to_string())).await.unwrap();
    assert!(response.0.success);
    assert_eq!(response.0.data.unwrap().id, id);
}

#[tokio::test]
async fn test_get_map_invalid_id() {
    let (handlers, _registry, _dirs) = setup_api_fixture().await;

    let result = handlers.get_map(Path("not-a-number".to_string())).await;
    assert_eq!(
        result.unwrap_err().status_code(),
        axum::http::StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn test_install_map_validation_both_sources() {
    let (handlers, _registry, _dirs) = setup_api_fixture().await;

    let result = handlers
        .install_map(Json(InstallMapRequest {
            url: Some("https://example.com/map.zip".to_string()),
            workshop_id: Some(123),
            name: None,
        }))
        .await;

    assert_eq!(
        result.unwrap_err().status_code(),
        axum::http::StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn test_install_map_validation_neither_source() {
    let (handlers, _registry, _dirs) = setup_api_fixture().await;

    let result = handlers
        .install_map(Json(InstallMapRequest {
            url: None,
            workshop_id: None,
            name: None,
        }))
        .await;

    assert_eq!(
        result.unwrap_err().status_code(),
        axum::http::StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn test_uninstall_map_success() {
    let (handlers, registry, _dirs) = setup_api_fixture().await;
    let id = registry.add_map(sample_map()).await.unwrap();

    let response = handlers.uninstall_map(Path(id.to_string())).await.unwrap();
    assert!(response.0.success);
    assert!(registry.get_map(id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_uninstall_map_invalid_id() {
    let (handlers, _registry, _dirs) = setup_api_fixture().await;

    let result = handlers.uninstall_map(Path("abc".to_string())).await;
    assert_eq!(
        result.unwrap_err().status_code(),
        axum::http::StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn test_uninstall_map_not_found() {
    let (handlers, _registry, _dirs) = setup_api_fixture().await;

    let result = handlers.uninstall_map(Path("99999".to_string())).await;
    assert_eq!(
        result.unwrap_err().status_code(),
        axum::http::StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn test_get_map_hides_denylisted() {
    use std::sync::Arc;

    use crate::api::handlers::ApiHandlers;
    use crate::config::init_handle;
    use crate::map_installer::MapInstallationService;
    use crate::registry::models::SourceKind;

    let (registry, dirs) = crate::test_helpers::setup_test_dirs().await.unwrap();
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

    let hidden_id = registry
        .add_map(MapEntry {
            id: 0,
            name: "Hidden".to_string(),
            source_url: String::new(),
            source_kind: SourceKind::Other,
            workshop_id: None,
            installed_path: "hidden.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: None,
            checksum: None,
            checksum_kind: None,
        })
        .await
        .unwrap();

    let mut config = crate::config::Config::default();
    config.hidden_map_ids = vec![hidden_id];
    let handlers = ApiHandlers::new(registry, installer, init_handle(config));

    let result = handlers.get_map(Path(hidden_id.to_string())).await;
    assert_eq!(
        result.unwrap_err().status_code(),
        axum::http::StatusCode::NOT_FOUND
    );
}
