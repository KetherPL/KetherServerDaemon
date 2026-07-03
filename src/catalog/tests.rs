// SPDX-License-Identifier: GPL-3.0-only
use std::sync::Arc;

use crate::catalog::{enrich_with_registry, CatalogMapStatus, L4d2CenterIndexEntry};
use crate::registry::{JsonRegistry, Registry};

const SAMPLE_INDEX: &str = r#"[
    {
        "name": "widebox1.vpk",
        "size": 10166415,
        "md5": "b0bd409a4bbc4a61ae000c73a9aa9934",
        "download_link": "https://l4d2center.com/maps/servers/widebox1.7z"
    }
]"#;

#[tokio::test]
async fn enrich_with_registry_marks_installed_and_outdated() {
    let dir = tempfile::TempDir::new().unwrap();
    let registry_path = dir.path().join("registry.json");
    let registry = Arc::new(JsonRegistry::new(&registry_path).await.unwrap()) as Arc<dyn Registry>;

    let entry = crate::registry::models::MapEntry {
        id: 0,
        name: "Widebox".to_string(),
        source_url: "https://l4d2center.com/maps/servers/widebox1.7z".to_string(),
        source_kind: crate::registry::models::SourceKind::L4d2Center,
        workshop_id: None,
        installed_path: "widebox1.vpk".to_string(),
        installed_at: chrono::Utc::now(),
        workshop_updated_at: None,
        version: None,
        checksum: Some("deadbeef".to_string()),
        checksum_kind: Some("md5".to_string()),
    };
    registry.add_map(entry).await.unwrap();

    let index: Vec<L4d2CenterIndexEntry> = serde_json::from_str(SAMPLE_INDEX).unwrap();
    let catalog = enrich_with_registry(index, registry.as_ref())
        .await
        .unwrap();

    assert_eq!(catalog.len(), 1);
    assert!(catalog[0].installed);
    assert_eq!(catalog[0].map_id, Some(1));
    assert_eq!(catalog[0].status, CatalogMapStatus::Outdated);
}

#[tokio::test]
async fn enrich_with_registry_marks_other_source_when_not_l4d2center() {
    let dir = tempfile::TempDir::new().unwrap();
    let registry_path = dir.path().join("registry.json");
    let registry = Arc::new(JsonRegistry::new(&registry_path).await.unwrap()) as Arc<dyn Registry>;

    let entry = crate::registry::models::MapEntry {
        id: 0,
        name: "No Echo".to_string(),
        source_url: "https://steamcommunity.com/sharedfiles/filedetails/?id=12345".to_string(),
        source_kind: crate::registry::models::SourceKind::Workshop,
        workshop_id: Some(12345),
        installed_path: "widebox1.vpk".to_string(),
        installed_at: chrono::Utc::now(),
        workshop_updated_at: None,
        version: None,
        checksum: Some("deadbeef".to_string()),
        checksum_kind: Some("md5".to_string()),
    };
    registry.add_map(entry).await.unwrap();

    let index: Vec<L4d2CenterIndexEntry> = serde_json::from_str(SAMPLE_INDEX).unwrap();
    let catalog = enrich_with_registry(index, registry.as_ref())
        .await
        .unwrap();

    assert_eq!(catalog.len(), 1);
    assert!(catalog[0].installed);
    assert_eq!(catalog[0].map_id, Some(1));
    assert_eq!(catalog[0].status, CatalogMapStatus::OtherSource);
}
