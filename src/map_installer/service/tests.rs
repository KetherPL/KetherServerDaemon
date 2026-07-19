use super::*;
use chrono::TimeZone;
use crate::registry::models::SourceKind;
use crate::registry::traits::Registry;
use crate::test_helpers;
use std::sync::Arc;
use tempfile::TempDir;
use zip::write::{FileOptions, ZipWriter};
use zip::CompressionMethod;
use std::io::Write;

async fn setup_test_service() -> (MapInstallationService, Arc<dyn Registry>, test_helpers::TestDirs) {
        let (registry, dirs) = test_helpers::setup_test_dirs().await.unwrap();
        let paths = dirs.service_paths();

        let service = MapInstallationService::new(
            Arc::clone(&registry),
            paths.addons_dir,
            paths.download_dir,
            100 * 1024 * 1024,
            1024 * 1024 * 1024,
            10000,
        )
        .await
        .unwrap();

        (service, registry, dirs)
    }

    fn create_test_zip_with_map(contents: &[(&str, &[u8])]) -> (PathBuf, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let zip_path = temp_dir.path().join("test_map.zip");
        
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = ZipWriter::new(file);
        
        for (name, data) in contents {
            zip.start_file(*name, FileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
            zip.write_all(data).unwrap();
        }
        
        zip.finish().unwrap();
        (zip_path, temp_dir)
    }

    #[tokio::test]
    async fn test_registry_accessor() {
        let (service, registry, _dirs) = setup_test_service().await;
        // Verify we can access the registry
        let service_registry = service.registry();
        assert_eq!(Arc::as_ptr(service_registry), Arc::as_ptr(&registry));
    }

    #[tokio::test]
    async fn test_uninstall_map_exists() {
        let (service, registry, _dirs) = setup_test_service().await;
        
        // Add a test map entry
        let mut map_entry = MapEntry {
            id: 0, // Will be assigned by database
            name: "Test Map".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            source_kind: SourceKind::Other,
            workshop_id: None,
            installed_path: "test_map.vpk".to_string(), // Relative path
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: None,
            checksum: None,
            checksum_kind: None,
        };
        let assigned_id = registry.add_map(map_entry.clone()).await.unwrap();
        map_entry.id = assigned_id;
        
        // Uninstall should succeed even if path doesn't exist
        let result = service.uninstall_map(assigned_id).await;
        assert!(result.is_ok());
        
        // Verify map was removed from registry
        let retrieved = registry.get_map(assigned_id).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_uninstall_map_not_exists() {
        let (service, _registry, _dirs) = setup_test_service().await;

        let result = service.uninstall_map(99999).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_uninstall_map_removes_file_before_registry() {
        let (service, registry, dirs) = setup_test_service().await;
        let vpk_path = dirs.addons_path().join("present.vpk");
        tokio::fs::write(&vpk_path, b"vpk-bytes").await.unwrap();

        let map_entry = MapEntry {
            id: 0,
            name: "Present".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            source_kind: SourceKind::Other,
            workshop_id: None,
            installed_path: "present.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: None,
            checksum: None,
            checksum_kind: None,
        };
        let assigned_id = registry.add_map(map_entry).await.unwrap();

        let result = service.uninstall_map(assigned_id).await;
        assert!(result.is_ok());
        assert!(!vpk_path.exists());
        assert!(registry.get_map(assigned_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_install_from_url_rejects_invalid_url() {
        let (service, _registry, _dirs) = setup_test_service().await;
        
        // This should fail because numeric strings are not valid URLs
        let result = service.install_from_url("123456789".to_string(), None).await;
        assert!(result.is_err());
        
        // Error should indicate URL validation failure
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("Invalid URL") || 
            error_msg.contains("SSRF") ||
            error_msg.contains("scheme")
        );
    }

    #[tokio::test]
    async fn test_install_from_url_dispatch_zip() {
        let (service, registry, _dirs) = setup_test_service().await;

        let vpk_temp = TempDir::new().unwrap();
        let vpk_path = vpk_temp.path().join("test_map.vpk");
        test_helpers::write_minimal_test_vpk(&vpk_path, "Test Map").unwrap();
        let vpk_bytes = std::fs::read(&vpk_path).unwrap();
        let (test_zip_path, _zip_temp) = create_test_zip_with_map(&[("test_map.vpk", &vpk_bytes)]);

        // Exercise the ZIP install path directly; HTTP download is covered by downloader tests.
        // install_from_url rejects localhost mock URLs via SSRF validation by design.
        let result = service
            .install_downloaded_file(
                test_zip_path,
                SourceKind::Other,
                None,
                Some("Test Map".to_string()),
                Some("https://example.com/test_map.zip".to_string()),
                None,
            )
            .await
            .expect("ZIP install should succeed");

        assert_eq!(result.name, "test_map");
        assert_eq!(result.source_kind, SourceKind::Other);
        assert!(result.installed_path.ends_with(".vpk"));

        let retrieved = registry.get_map(result.id).await.unwrap();
        assert!(retrieved.is_some());
    }

    #[tokio::test]
    async fn test_compact_registry_prunes_sorts_and_reindexes() {
        let (service, registry, dirs) = setup_test_service().await;

        tokio::fs::write(dirs.addons_path().join("alpha.vpk"), b"vpk").await.unwrap();
        tokio::fs::write(dirs.addons_path().join("zulu.vpk"), b"vpk").await.unwrap();

        let now = chrono::Utc::now();
        registry
            .replace_all_maps(vec![
                MapEntry {
                    id: 5,
                    name: "Zulu".to_string(),
                    source_url: "https://example.com/zulu".to_string(),
                    source_kind: SourceKind::Other,
                    workshop_id: None,
                    installed_path: "zulu.vpk".to_string(),
                    installed_at: now,
                    workshop_updated_at: None,
                    version: None,
                    checksum: None,
                    checksum_kind: None,
                },
                MapEntry {
                    id: 12,
                    name: "Alpha".to_string(),
                    source_url: "https://example.com/alpha".to_string(),
                    source_kind: SourceKind::Other,
                    workshop_id: None,
                    installed_path: "alpha.vpk".to_string(),
                    installed_at: now,
                    workshop_updated_at: None,
                    version: None,
                    checksum: None,
                    checksum_kind: None,
                },
                MapEntry {
                    id: 3,
                    name: "Missing".to_string(),
                    source_url: "https://example.com/missing".to_string(),
                    source_kind: SourceKind::Other,
                    workshop_id: None,
                    installed_path: "missing.vpk".to_string(),
                    installed_at: now,
                    workshop_updated_at: None,
                    version: None,
                    checksum: None,
                    checksum_kind: None,
                },
            ])
            .await
            .unwrap();

        let report = service.compact_registry().await.unwrap();

        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].name, "Missing");
        assert_eq!(report.kept.len(), 2);
        assert_eq!(report.kept[0].name, "Alpha");
        assert_eq!(report.kept[0].id, 1);
        assert_eq!(report.kept[1].name, "Zulu");
        assert_eq!(report.kept[1].id, 2);

        let maps = registry.list_maps().await.unwrap();
        assert_eq!(maps.len(), 2);
        assert_eq!(maps[0].id, 1);
        assert_eq!(maps[0].name, "Alpha");
        assert_eq!(maps[1].id, 2);
        assert_eq!(maps[1].name, "Zulu");
    }

    fn create_modify_test_entry() -> MapEntry {
        MapEntry {
            id: 0,
            name: "Test Map".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            source_kind: SourceKind::Workshop,
            workshop_id: Some(999),
            installed_path: "test_map.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: Some("1.0".to_string()),
            checksum: None,
            checksum_kind: None,
        }
    }

    #[tokio::test]
    async fn test_modify_map_field_workshop_id_sets_kind_and_url() {
        let (service, registry, _dirs) = setup_test_service().await;
        let id = registry.add_map(create_modify_test_entry()).await.unwrap();

        let updated = service
            .modify_map_field(id, "workshop_id", "3135451698")
            .await
            .unwrap();

        assert_eq!(updated.workshop_id, Some(3135451698));
        assert_eq!(updated.source_kind, SourceKind::Workshop);
        assert_eq!(
            updated.source_url,
            "https://steamcommunity.com/sharedfiles/filedetails/?id=3135451698"
        );

        let retrieved = registry.get_map(id).await.unwrap().unwrap();
        assert_eq!(retrieved.workshop_id, Some(3135451698));
        assert_eq!(retrieved.source_kind, SourceKind::Workshop);
        assert_eq!(
            retrieved.source_url,
            "https://steamcommunity.com/sharedfiles/filedetails/?id=3135451698"
        );
    }

    #[tokio::test]
    async fn test_modify_map_field_source_kind_other_clears_workshop_id() {
        let (service, registry, _dirs) = setup_test_service().await;
        let id = registry.add_map(create_modify_test_entry()).await.unwrap();

        let updated = service
            .modify_map_field(id, "source_kind", "other")
            .await
            .unwrap();

        assert_eq!(updated.source_kind, SourceKind::Other);
        assert_eq!(updated.workshop_id, None);

        let retrieved = registry.get_map(id).await.unwrap().unwrap();
        assert_eq!(retrieved.source_kind, SourceKind::Other);
        assert_eq!(retrieved.workshop_id, None);
    }

    #[tokio::test]
    async fn test_modify_map_field_unknown_field_errors() {
        let (service, registry, _dirs) = setup_test_service().await;
        let id = registry.add_map(create_modify_test_entry()).await.unwrap();

        let result = service
            .modify_map_field(id, "checksum", "abc123")
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unknown or read-only field"));
    }

    #[tokio::test]
    async fn test_modify_installed_path_renames_file_and_updates_registry() {
        let (service, registry, dirs) = setup_test_service().await;
        let addons = dirs.addons_path();
        let old_path = addons.join("test_map.vpk");
        test_helpers::write_minimal_test_vpk(&old_path, "Test Map").unwrap();

        let id = registry.add_map(create_modify_test_entry()).await.unwrap();

        let updated = service
            .modify_map_field(id, "installed_path", "renamed.vpk")
            .await
            .unwrap();

        assert_eq!(updated.installed_path, "renamed.vpk");
        assert!(!old_path.exists());
        assert!(addons.join("renamed.vpk").exists());

        let retrieved = registry.get_map(id).await.unwrap().unwrap();
        assert_eq!(retrieved.installed_path, "renamed.vpk");
    }

    #[tokio::test]
    async fn test_modify_installed_path_workshop_subdir() {
        let (service, registry, dirs) = setup_test_service().await;
        let addons = dirs.addons_path();
        let old_path = addons.join("test_map.vpk");
        test_helpers::write_minimal_test_vpk(&old_path, "Test Map").unwrap();

        let id = registry.add_map(create_modify_test_entry()).await.unwrap();

        let updated = service
            .modify_map_field(id, "installed_path", "workshop/renamed.vpk")
            .await
            .unwrap();

        assert_eq!(updated.installed_path, "workshop/renamed.vpk");
        assert!(!old_path.exists());
        assert!(addons.join("workshop/renamed.vpk").exists());
    }

    #[tokio::test]
    async fn test_modify_installed_path_registry_only_when_source_missing() {
        let (service, registry, _dirs) = setup_test_service().await;
        let id = registry.add_map(create_modify_test_entry()).await.unwrap();

        let updated = service
            .modify_map_field(id, "installed_path", "renamed.vpk")
            .await
            .unwrap();

        assert_eq!(updated.installed_path, "renamed.vpk");

        let retrieved = registry.get_map(id).await.unwrap().unwrap();
        assert_eq!(retrieved.installed_path, "renamed.vpk");
    }

    #[tokio::test]
    async fn test_modify_installed_path_rejects_conflict() {
        let (service, registry, dirs) = setup_test_service().await;
        let addons = dirs.addons_path();
        test_helpers::write_minimal_test_vpk(&addons.join("test_map.vpk"), "Test Map").unwrap();
        test_helpers::write_minimal_test_vpk(&addons.join("taken.vpk"), "Taken").unwrap();

        let id = registry.add_map(create_modify_test_entry()).await.unwrap();

        let result = service
            .modify_map_field(id, "installed_path", "taken.vpk")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("file already exists"));

        let mut other = create_modify_test_entry();
        other.installed_path = "other_map.vpk".to_string();
        let other_id = registry.add_map(other).await.unwrap();

        let result = service
            .modify_map_field(id, "installed_path", "other_map.vpk")
            .await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains(&format!("already used by map #{other_id}")));
    }

    #[tokio::test]
    async fn test_modify_map_field_missing_id_errors() {
        let (service, _registry, _dirs) = setup_test_service().await;

        let result = service
            .modify_map_field(99999, "name", "New Name")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Map not found"));
    }

    #[test]
    fn test_preserve_source_identity_keeps_existing_when_fresh_has_no_workshop_id() {
        let existing = MapEntry {
            id: 1,
            name: "Existing".to_string(),
            source_url: "https://steamcommunity.com/sharedfiles/filedetails/?id=12345".to_string(),
            source_kind: SourceKind::Workshop,
            workshop_id: Some(12345),
            installed_path: "map.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: Some("1".to_string()),
            checksum: Some("old".to_string()),
            checksum_kind: Some("md5".to_string()),
        };

        let mut fresh = MapEntry {
            id: 1,
            name: "Fresh".to_string(),
            source_url: "detected:/addons/map.vpk".to_string(),
            source_kind: SourceKind::Other,
            workshop_id: None,
            installed_path: "map.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: Some("2".to_string()),
            checksum: Some("new".to_string()),
            checksum_kind: Some("md5".to_string()),
        };

        MapInstallationService::preserve_source_identity(&mut fresh, &existing);

        assert_eq!(fresh.workshop_id, Some(12345));
        assert_eq!(fresh.source_kind, SourceKind::Workshop);
        assert_eq!(
            fresh.source_url,
            "https://steamcommunity.com/sharedfiles/filedetails/?id=12345"
        );
        assert_eq!(fresh.name, "Fresh");
        assert_eq!(fresh.version, Some("2".to_string()));
        assert_eq!(fresh.checksum, Some("new".to_string()));
    }

    #[test]
    fn test_preserve_source_identity_uses_fresh_when_workshop_id_detected() {
        let existing = MapEntry {
            id: 1,
            name: "Existing".to_string(),
            source_url: "detected:/addons/map.vpk".to_string(),
            source_kind: SourceKind::Other,
            workshop_id: None,
            installed_path: "map.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: Some("1".to_string()),
            checksum: Some("old".to_string()),
            checksum_kind: Some("md5".to_string()),
        };

        let mut fresh = MapEntry {
            id: 1,
            name: "Fresh".to_string(),
            source_url: "https://steamcommunity.com/sharedfiles/filedetails/?id=999".to_string(),
            source_kind: SourceKind::Workshop,
            workshop_id: Some(999),
            installed_path: "map.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: Some("2".to_string()),
            checksum: Some("new".to_string()),
            checksum_kind: Some("md5".to_string()),
        };

        MapInstallationService::preserve_source_identity(&mut fresh, &existing);

        assert_eq!(fresh.workshop_id, Some(999));
        assert_eq!(fresh.source_kind, SourceKind::Workshop);
        assert_eq!(
            fresh.source_url,
            "https://steamcommunity.com/sharedfiles/filedetails/?id=999"
        );
    }

    #[tokio::test]
    async fn test_detect_map_from_path_is_idempotent_for_installed_path() {
        let (service, registry, dirs) = setup_test_service().await;
        tokio::fs::write(dirs.addons_path().join("alpha.vpk"), b"vpk")
            .await
            .unwrap();

        let id = registry
            .add_map(MapEntry {
                id: 0,
                name: "Alpha".to_string(),
                source_url: "https://example.com/alpha".to_string(),
                source_kind: SourceKind::Other,
                workshop_id: None,
                installed_path: "alpha.vpk".to_string(),
                installed_at: chrono::Utc::now(),
                workshop_updated_at: None,
                version: None,
                checksum: None,
                checksum_kind: None,
            })
            .await
            .unwrap();

        let path = dirs.addons_path().join("alpha.vpk");
        let result = service.detect_map_from_path(path).await.unwrap().unwrap();
        assert_eq!(result.id, id);
        assert_eq!(registry.list_maps().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_sync_map_from_path_returns_existing_when_metadata_unavailable() {
        let (service, registry, dirs) = setup_test_service().await;
        tokio::fs::write(dirs.addons_path().join("alpha.vpk"), b"vpk")
            .await
            .unwrap();

        let id = registry
            .add_map(MapEntry {
                id: 0,
                name: "Alpha".to_string(),
                source_url: "https://example.com/alpha".to_string(),
                source_kind: SourceKind::Other,
                workshop_id: None,
                installed_path: "alpha.vpk".to_string(),
                installed_at: chrono::Utc::now(),
                workshop_updated_at: None,
                version: None,
                checksum: Some("old".to_string()),
                checksum_kind: Some("md5".to_string()),
            })
            .await
            .unwrap();

        let result = service
            .sync_map_from_path(dirs.addons_path().join("alpha.vpk"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.id, id);
        assert_eq!(registry.list_maps().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_build_map_entry_from_file_uses_filename_fallback_when_metadata_missing() {
        let (service, _registry, dirs) = setup_test_service().await;
        let path = dirs.addons_path().join("bts_l4d2.vpk");
        tokio::fs::write(&path, b"not a vpk").await.unwrap();

        let entry = service
            .build_map_entry_from_file(&path, "bts_l4d2.vpk")
            .await
            .unwrap()
            .expect("fallback entry");

        assert_eq!(entry.name, "bts_l4d2");
        assert_eq!(entry.installed_path, "bts_l4d2.vpk");
        assert!(entry.version.is_none());
        assert_eq!(entry.source_kind, SourceKind::Other);
    }

    #[tokio::test]
    async fn test_remove_map_by_path_prunes_when_file_gone() {
        let (service, registry, dirs) = setup_test_service().await;
        let id = registry
            .add_map(MapEntry {
                id: 0,
                name: "Gone".to_string(),
                source_url: "https://example.com/gone".to_string(),
                source_kind: SourceKind::Other,
                workshop_id: None,
                installed_path: "gone.vpk".to_string(),
                installed_at: chrono::Utc::now(),
                workshop_updated_at: None,
                version: None,
                checksum: None,
                checksum_kind: None,
            })
            .await
            .unwrap();

        let removed = service
            .remove_map_by_path(dirs.addons_path().join("gone.vpk"))
            .await
            .unwrap();
        assert_eq!(removed, Some(id));
        assert!(registry.get_map(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_remove_map_by_path_noop_when_file_exists() {
        let (service, registry, dirs) = setup_test_service().await;
        tokio::fs::write(dirs.addons_path().join("exists.vpk"), b"vpk")
            .await
            .unwrap();

        let id = registry
            .add_map(MapEntry {
                id: 0,
                name: "Exists".to_string(),
                source_url: "https://example.com/exists".to_string(),
                source_kind: SourceKind::Other,
                workshop_id: None,
                installed_path: "exists.vpk".to_string(),
                installed_at: chrono::Utc::now(),
                workshop_updated_at: None,
                version: None,
                checksum: None,
                checksum_kind: None,
            })
            .await
            .unwrap();

        let removed = service
            .remove_map_by_path(dirs.addons_path().join("exists.vpk"))
            .await
            .unwrap();
        assert_eq!(removed, None);
        assert!(registry.get_map(id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_install_workshop_id_returns_existing_without_redownload() {
        let (service, registry, _dirs) = setup_test_service().await;
        let workshop_id = 3135451698u64;
        let existing_id = registry
            .add_map(MapEntry {
                id: 0,
                name: "Workshop Map".to_string(),
                source_url: String::new(),
                source_kind: SourceKind::Workshop,
                workshop_id: Some(workshop_id),
                installed_path: "workshop_map.vpk".to_string(),
                installed_at: chrono::Utc::now(),
                workshop_updated_at: None,
                version: None,
                checksum: None,
                checksum_kind: None,
            })
            .await
            .unwrap();

        let result = service
            .install_from_workshop_id(workshop_id, None)
            .await
            .unwrap();
        assert_eq!(result.id, existing_id);
        assert_eq!(registry.list_maps().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_find_map_by_name_returns_existing_entry() {
        let (service, registry, _dirs) = setup_test_service().await;
        let id = registry
            .add_map(MapEntry {
                id: 0,
                name: "existing_map".to_string(),
                source_url: "https://example.com/map".to_string(),
                source_kind: SourceKind::Other,
                workshop_id: None,
                installed_path: "existing.vpk".to_string(),
                installed_at: chrono::Utc::now(),
                workshop_updated_at: None,
                version: None,
                checksum: None,
                checksum_kind: None,
            })
            .await
            .unwrap();

        let found = service.find_map_by_name("existing_map").await.unwrap();
        assert_eq!(found.unwrap().id, id);
    }

    #[test]
    fn test_needs_workshop_update_stored_timestamp() {
        let steam = chrono::Utc.timestamp_opt(2_000, 0).single().unwrap();
        let stored = chrono::Utc.timestamp_opt(1_000, 0).single().unwrap();
        assert!(needs_workshop_update(steam, Some(stored), None, false));
        assert!(!needs_workshop_update(stored, Some(steam), None, false));
    }

    #[test]
    fn test_needs_workshop_update_mtime_fallback() {
        let steam = chrono::Utc.timestamp_opt(2_000, 0).single().unwrap();
        let mtime = chrono::Utc.timestamp_opt(1_000, 0).single().unwrap();
        assert!(needs_workshop_update(steam, None, Some(mtime), false));
        assert!(!needs_workshop_update(mtime, None, Some(steam), false));
    }

    #[test]
    fn test_needs_workshop_update_no_signals_defaults_outdated() {
        let steam = chrono::Utc.timestamp_opt(1_000, 0).single().unwrap();
        assert!(needs_workshop_update(steam, None, None, false));
    }

    #[test]
    fn test_needs_workshop_update_force() {
        let steam = chrono::Utc.timestamp_opt(1_000, 0).single().unwrap();
        let stored = chrono::Utc.timestamp_opt(9_000, 0).single().unwrap();
        assert!(needs_workshop_update(steam, Some(stored), None, true));
    }

    #[tokio::test]
    async fn test_workshop_update_file_copy_overwrites_target() {
        let (service, _registry, dirs) = setup_test_service().await;
        let downloaded = dirs.addons_path().join("workshop.vpk");
        let payload = b"downloaded-workshop-bytes";
        tokio::fs::write(&downloaded, payload).await.unwrap();

        let target = dirs.addons_path().join("installed.vpk");
        tokio::fs::write(&target, b"old-bytes").await.unwrap();

        let (source_vpk, cleanup) = service
            .prepare_vpk_from_download(downloaded.clone())
            .await
            .unwrap();
        tokio::fs::copy(&source_vpk, &target).await.unwrap();
        cleanup.cleanup().await;

        let on_disk = tokio::fs::read(&target).await.unwrap();
        assert_eq!(on_disk, payload);
    }

    #[tokio::test]
    async fn test_workshop_check_only_skips_non_workshop_md5() {
        let (service, registry, dirs) = setup_test_service().await;

        // Large-ish file that would be expensive to MD5 if resolve ran.
        let rel = "l4d2center/big.vpk";
        let path = dirs.addons_path().join(rel);
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        let chunk = vec![0u8; 256 * 1024];
        let mut file = tokio::fs::File::create(&path).await.unwrap();
        for _ in 0..8 {
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
                .await
                .unwrap();
        }
        drop(file);

        let map_entry = MapEntry {
            id: 0,
            name: "L4D2Center Map".to_string(),
            source_url: "https://example.com/big.zip".to_string(),
            source_kind: SourceKind::L4d2Center,
            workshop_id: None,
            installed_path: rel.to_string(),
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: None,
            checksum: None,
            checksum_kind: None,
        };
        registry.add_map(map_entry).await.unwrap();

        let started = std::time::Instant::now();
        let report = service
            .update_workshop_maps(None, false, true)
            .await
            .unwrap();
        // Without the skip, MD5 of ~2MiB would still be fast locally, but we assert
        // the map was classified as not_workshop without becoming a candidate.
        assert_eq!(report.not_workshop, 1);
        assert!(report.available.is_empty());
        assert!(report.failed.is_empty());
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

