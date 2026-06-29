// SPDX-License-Identifier: GPL-3.0-only
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use tokio::sync::Mutex;
use tracing::info;

use crate::registry::{
    models::{MapEntry, SourceKind},
    traits::Registry,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MapData {
    name: String,
    source_url: String,
    source_kind: SourceKind,
    workshop_id: Option<u64>,
    installed_path: String,
    installed_at: DateTime<Utc>,
    version: Option<String>,
    checksum: Option<String>,
    checksum_kind: Option<String>,
}

struct NumericOrderedSnapshot<'a>(&'a [(u64, &'a MapData)]);

impl Serialize for NumericOrderedSnapshot<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (id, data) in self.0 {
            map.serialize_entry(&id.to_string(), data)?;
        }
        map.end()
    }
}

pub struct JsonRegistry {
    inner: Arc<RwLock<HashMap<u64, MapData>>>,
    path: PathBuf,
    save_lock: Mutex<()>,
}

impl JsonRegistry {
    pub async fn new(path: &PathBuf) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!(
                    "Failed to create parent directory for registry file: {}",
                    parent.display()
                )
            })?;
        }

        let map = if path.exists() {
            Self::load_from_file(path).await?
        } else {
            HashMap::new()
        };

        let registry = Self {
            inner: Arc::new(RwLock::new(map)),
            path: path.clone(),
            save_lock: Mutex::new(()),
        };

        if !path.exists() {
            registry.persist().await?;
        }

        Ok(registry)
    }

    async fn load_from_file(path: &PathBuf) -> anyhow::Result<HashMap<u64, MapData>> {
        let content = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("Failed to read registry file at {}", path.display()))?;

        if content.trim().is_empty() {
            return Ok(HashMap::new());
        }

        let raw_map: HashMap<String, MapData> = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse registry JSON at {}", path.display()))?;

        let mut parsed = HashMap::with_capacity(raw_map.len());
        for (id, entry) in raw_map {
            let parsed_id = id
                .parse::<u64>()
                .with_context(|| format!("Invalid registry id key '{id}' in {}", path.display()))?;
            parsed.insert(parsed_id, entry);
        }

        Ok(parsed)
    }

    async fn persist(&self) -> anyhow::Result<()> {
        let _guard = self.save_lock.lock().await;
        let snapshot = {
            let state = self
                .inner
                .read()
                .map_err(|e| anyhow::anyhow!("Registry lock poisoned: {e}"))?;
            state.clone()
        };
        Self::save_snapshot(&self.path, &snapshot).await
    }

    async fn save_snapshot(path: &PathBuf, snapshot: &HashMap<u64, MapData>) -> anyhow::Result<()> {
        static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

        let mut entries: Vec<(u64, &MapData)> =
            snapshot.iter().map(|(id, data)| (*id, data)).collect();
        entries.sort_by_key(|(id, _)| *id);

        let json = serde_json::to_string_pretty(&NumericOrderedSnapshot(&entries))
            .with_context(|| format!("Failed to serialize registry JSON for {}", path.display()))?;

        let temp_suffix = format!("tmp.{}", TEMP_COUNTER.fetch_add(1, Ordering::Relaxed));
        let temp_path = path.with_extension(temp_suffix);
        tokio::fs::write(&temp_path, json)
            .await
            .with_context(|| format!("Failed to write temp registry file {}", temp_path.display()))?;
        tokio::fs::rename(&temp_path, path).await.with_context(|| {
            format!(
                "Failed to rename temp registry file {} to {}",
                temp_path.display(),
                path.display()
            )
        })?;

        Ok(())
    }

    fn normalize_workshop_id(source_kind: SourceKind, workshop_id: Option<u64>) -> Option<u64> {
        if source_kind == SourceKind::Workshop {
            workshop_id
        } else {
            None
        }
    }

    fn map_data_from_entry(entry: MapEntry) -> MapData {
        let workshop_id = Self::normalize_workshop_id(entry.source_kind, entry.workshop_id);
        MapData {
            name: entry.name,
            source_url: entry.source_url,
            source_kind: entry.source_kind,
            workshop_id,
            installed_path: entry.installed_path,
            installed_at: entry.installed_at,
            version: entry.version,
            checksum: entry.checksum,
            checksum_kind: entry.checksum_kind,
        }
    }

    fn map_entry_from_data(id: u64, data: &MapData) -> MapEntry {
        MapEntry {
            id,
            name: data.name.clone(),
            source_url: data.source_url.clone(),
            source_kind: data.source_kind,
            workshop_id: Self::normalize_workshop_id(data.source_kind, data.workshop_id),
            installed_path: data.installed_path.clone(),
            installed_at: data.installed_at.clone(),
            version: data.version.clone(),
            checksum: data.checksum.clone(),
            checksum_kind: data.checksum_kind.clone(),
        }
    }
}

#[async_trait]
impl Registry for JsonRegistry {
    async fn add_map(&self, mut entry: MapEntry) -> anyhow::Result<u64> {
        let id = {
            let mut state = self
                .inner
                .write()
                .map_err(|e| anyhow::anyhow!("Registry lock poisoned: {e}"))?;
            let id = state.keys().max().copied().unwrap_or(0) + 1;
            entry.id = id;
            state.insert(id, Self::map_data_from_entry(entry));
            id
        };

        self.persist().await?;
        info!(map_id = id, "Added map to JSON registry");
        Ok(id)
    }

    async fn remove_map(&self, id: u64) -> anyhow::Result<()> {
        let removed = {
            let mut state = self
                .inner
                .write()
                .map_err(|e| anyhow::anyhow!("Registry lock poisoned: {e}"))?;
            state.remove(&id).is_some()
        };

        if !removed {
            return Ok(());
        }

        self.persist().await?;
        info!(map_id = id, "Removed map from JSON registry");
        Ok(())
    }

    async fn get_map(&self, id: u64) -> anyhow::Result<Option<MapEntry>> {
        let state = self
            .inner
            .read()
            .map_err(|e| anyhow::anyhow!("Registry lock poisoned: {e}"))?;
        Ok(state.get(&id).map(|entry| Self::map_entry_from_data(id, entry)))
    }

    async fn list_maps(&self) -> anyhow::Result<Vec<MapEntry>> {
        let state = self
            .inner
            .read()
            .map_err(|e| anyhow::anyhow!("Registry lock poisoned: {e}"))?;

        let mut ids: Vec<u64> = state.keys().copied().collect();
        ids.sort_unstable();
        let maps = ids
            .into_iter()
            .filter_map(|id| state.get(&id).map(|entry| Self::map_entry_from_data(id, entry)))
            .collect();

        Ok(maps)
    }

    async fn update_map(&self, entry: MapEntry) -> anyhow::Result<()> {
        let updated = {
            let mut state = self
                .inner
                .write()
                .map_err(|e| anyhow::anyhow!("Registry lock poisoned: {e}"))?;
            if !state.contains_key(&entry.id) {
                return Ok(());
            }
            let id = entry.id;
            state.insert(id, Self::map_data_from_entry(entry));
            true
        };

        if updated {
            self.persist().await?;
        }
        Ok(())
    }

    async fn replace_all_maps(&self, entries: Vec<MapEntry>) -> anyhow::Result<()> {
        {
            let mut state = self
                .inner
                .write()
                .map_err(|e| anyhow::anyhow!("Registry lock poisoned: {e}"))?;
            state.clear();
            for entry in entries {
                let id = entry.id;
                state.insert(id, Self::map_data_from_entry(entry));
            }
        }

        self.persist().await?;
        let count = self.list_maps().await?.len();
        info!(count, "Replaced all maps in JSON registry");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn create_test_map_entry(id: u64) -> MapEntry {
        MapEntry {
            id,
            name: "Test Map".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            source_kind: SourceKind::Workshop,
            workshop_id: Some(123456789),
            installed_path: "test_map.vpk".to_string(),
            installed_at: Utc::now(),
            version: Some("1.0.0".to_string()),
            checksum: Some("abc123def456".to_string()),
            checksum_kind: Some("md5".to_string()),
        }
    }

    async fn setup_test_registry() -> (TempDir, PathBuf, JsonRegistry) {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("registry.json");
        let registry = JsonRegistry::new(&path).await.unwrap();
        (temp_dir, path, registry)
    }

    #[tokio::test]
    async fn test_new_creates_missing_parent_and_registry_file() {
        let temp_dir = TempDir::new().unwrap();
        let registry_path = temp_dir.path().join("nested").join("registry.json");

        assert!(!registry_path.exists());
        let registry = JsonRegistry::new(&registry_path).await.unwrap();
        assert!(registry_path.exists());
        assert!(registry.list_maps().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_add_and_get_map() {
        let (_temp_dir, _path, registry) = setup_test_registry().await;
        let mut entry = create_test_map_entry(0);
        let assigned_id = registry.add_map(entry.clone()).await.unwrap();
        entry.id = assigned_id;

        let retrieved = registry.get_map(assigned_id).await.unwrap().unwrap();
        assert_eq!(retrieved.id, entry.id);
        assert_eq!(retrieved.name, entry.name);
        assert_eq!(retrieved.source_url, entry.source_url);
        assert_eq!(retrieved.workshop_id, entry.workshop_id);
    }

    #[tokio::test]
    async fn test_add_map_normalizes_workshop_id_for_non_workshop() {
        let (_temp_dir, _path, registry) = setup_test_registry().await;
        let entry = MapEntry {
            id: 0,
            name: "Non workshop".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            source_kind: SourceKind::Other,
            workshop_id: Some(12345),
            installed_path: "test_map.vpk".to_string(),
            installed_at: Utc::now(),
            version: None,
            checksum: None,
            checksum_kind: None,
        };

        let id = registry.add_map(entry).await.unwrap();
        let retrieved = registry.get_map(id).await.unwrap().unwrap();
        assert_eq!(retrieved.workshop_id, None);
    }

    #[tokio::test]
    async fn test_get_map_not_exists() {
        let (_temp_dir, _path, registry) = setup_test_registry().await;
        let retrieved = registry.get_map(99999).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_remove_map() {
        let (_temp_dir, _path, registry) = setup_test_registry().await;
        let entry = create_test_map_entry(0);
        let assigned_id = registry.add_map(entry).await.unwrap();

        assert!(registry.get_map(assigned_id).await.unwrap().is_some());
        registry.remove_map(assigned_id).await.unwrap();
        assert!(registry.get_map(assigned_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_remove_map_not_exists_is_ok() {
        let (_temp_dir, _path, registry) = setup_test_registry().await;
        registry.remove_map(99999).await.unwrap();
    }

    #[tokio::test]
    async fn test_list_maps_sorted_by_id() {
        let (_temp_dir, _path, registry) = setup_test_registry().await;
        let id1 = registry.add_map(create_test_map_entry(0)).await.unwrap();
        let id2 = registry.add_map(create_test_map_entry(0)).await.unwrap();
        let id3 = registry
            .add_map(MapEntry {
                id: 0,
                name: "Third".to_string(),
                source_url: "https://example.com/third.zip".to_string(),
                source_kind: SourceKind::Other,
                workshop_id: None,
                installed_path: "third.vpk".to_string(),
                installed_at: Utc::now(),
                version: None,
                checksum: None,
                checksum_kind: None,
            })
            .await
            .unwrap();

        let maps = registry.list_maps().await.unwrap();
        let ids: Vec<u64> = maps.into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![id1, id2, id3]);
    }

    #[tokio::test]
    async fn test_update_map() {
        let (_temp_dir, _path, registry) = setup_test_registry().await;
        let mut entry = create_test_map_entry(0);
        let assigned_id = registry.add_map(entry.clone()).await.unwrap();
        entry.id = assigned_id;
        entry.name = "Updated Map".to_string();
        entry.version = Some("2.0.0".to_string());

        registry.update_map(entry).await.unwrap();
        let retrieved = registry.get_map(assigned_id).await.unwrap().unwrap();
        assert_eq!(retrieved.name, "Updated Map");
        assert_eq!(retrieved.version, Some("2.0.0".to_string()));
    }

    #[tokio::test]
    async fn test_update_non_existent_map_is_ok() {
        let (_temp_dir, _path, registry) = setup_test_registry().await;
        let mut entry = create_test_map_entry(0);
        entry.id = 99999;
        registry.update_map(entry).await.unwrap();
    }

    #[tokio::test]
    async fn test_auto_increment_ids_start_at_one() {
        let (_temp_dir, _path, registry) = setup_test_registry().await;
        let id1 = registry.add_map(create_test_map_entry(0)).await.unwrap();
        let id2 = registry.add_map(create_test_map_entry(0)).await.unwrap();
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }

    #[tokio::test]
    async fn test_long_path_persists() {
        let (_temp_dir, _path, registry) = setup_test_registry().await;
        let long_path = "/".to_string() + &"a".repeat(500) + "/test/map";
        let entry = MapEntry {
            id: 0,
            name: "Long Path".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            source_kind: SourceKind::Other,
            workshop_id: None,
            installed_path: long_path.clone(),
            installed_at: Utc::now(),
            version: None,
            checksum: None,
            checksum_kind: None,
        };

        let id = registry.add_map(entry).await.unwrap();
        let retrieved = registry.get_map(id).await.unwrap().unwrap();
        assert_eq!(retrieved.installed_path, long_path);
    }

    #[tokio::test]
    async fn test_writes_id_keyed_json_object() {
        let (_temp_dir, path, registry) = setup_test_registry().await;
        let id = registry.add_map(create_test_map_entry(0)).await.unwrap();

        let raw = tokio::fs::read_to_string(&path).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let id_key = id.to_string();
        assert!(parsed.get(&id_key).is_some());
    }

    #[tokio::test]
    async fn test_replace_all_maps_reindexes_and_sorts() {
        let (_temp_dir, path, registry) = setup_test_registry().await;

        let mut alpha = create_test_map_entry(5);
        alpha.name = "Alpha".to_string();
        alpha.installed_path = "alpha.vpk".to_string();

        let mut bravo = create_test_map_entry(12);
        bravo.name = "Bravo".to_string();
        bravo.installed_path = "bravo.vpk".to_string();

        let mut charlie = create_test_map_entry(3);
        charlie.name = "Charlie".to_string();
        charlie.installed_path = "charlie.vpk".to_string();

        registry.add_map(alpha).await.unwrap();
        registry.add_map(bravo).await.unwrap();
        registry.add_map(charlie).await.unwrap();

        let reindexed = vec![
            MapEntry {
                id: 1,
                name: "Alpha".to_string(),
                ..create_test_map_entry(1)
            },
            MapEntry {
                id: 2,
                name: "Bravo".to_string(),
                installed_path: "bravo.vpk".to_string(),
                ..create_test_map_entry(2)
            },
            MapEntry {
                id: 3,
                name: "Charlie".to_string(),
                installed_path: "charlie.vpk".to_string(),
                ..create_test_map_entry(3)
            },
        ];

        registry.replace_all_maps(reindexed).await.unwrap();

        let maps = registry.list_maps().await.unwrap();
        let ids: Vec<u64> = maps.iter().map(|m| m.id).collect();
        let names: Vec<&str> = maps.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(ids, vec![1, 2, 3]);
        assert_eq!(names, vec!["Alpha", "Bravo", "Charlie"]);

        let raw = tokio::fs::read_to_string(&path).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(parsed.get("1").is_some());
        assert!(parsed.get("2").is_some());
        assert!(parsed.get("3").is_some());
        assert!(parsed.get("5").is_none());
        assert!(parsed.get("12").is_none());
    }

    #[tokio::test]
    async fn test_save_snapshot_serializes_keys_in_numeric_order() {
        let (_temp_dir, path, registry) = setup_test_registry().await;

        let entries: Vec<MapEntry> = (1..=12)
            .map(|id| MapEntry {
                id,
                name: format!("Map {id}"),
                ..create_test_map_entry(id)
            })
            .collect();

        registry.replace_all_maps(entries).await.unwrap();

        let raw = tokio::fs::read_to_string(&path).await.unwrap();
        let key_two = raw.find("\"2\":").expect("key 2 should be present");
        let key_ten = raw.find("\"10\":").expect("key 10 should be present");
        assert!(
            key_two < key_ten,
            "key \"2\" should appear before key \"10\" in numeric order"
        );
    }

    #[tokio::test]
    async fn test_concurrent_writes_persist_all_entries() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("registry.json");
        let registry = Arc::new(JsonRegistry::new(&path).await.unwrap());

        let mut handles = Vec::new();
        for i in 0..50 {
            let reg = Arc::clone(&registry);
            handles.push(tokio::spawn(async move {
                let entry = MapEntry {
                    id: 0,
                    name: format!("Map {i}"),
                    source_url: format!("https://example.com/{i}"),
                    source_kind: SourceKind::Other,
                    workshop_id: None,
                    installed_path: format!("map_{i}.vpk"),
                    installed_at: Utc::now(),
                    version: None,
                    checksum: None,
                    checksum_kind: None,
                };
                reg.add_map(entry).await.unwrap()
            }));
        }

        let mut ids = Vec::new();
        for handle in handles {
            ids.push(handle.await.unwrap());
        }
        assert_eq!(ids.len(), 50);

        let memory_maps = registry.list_maps().await.unwrap();
        assert_eq!(memory_maps.len(), 50);

        let reloaded = JsonRegistry::new(&path).await.unwrap();
        let disk_maps = reloaded.list_maps().await.unwrap();
        assert_eq!(disk_maps.len(), 50);
        let unique_ids: std::collections::HashSet<u64> = disk_maps.iter().map(|m| m.id).collect();
        assert_eq!(unique_ids.len(), 50);
    }
}
