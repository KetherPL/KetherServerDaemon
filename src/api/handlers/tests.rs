// SPDX-License-Identifier: GPL-3.0-only
use axum::extract::Path;
use axum::Json;

use crate::api::handlers::ApiHandlers;
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
