// SPDX-License-Identifier: GPL-3.0-only
//! Outgoing map representation: a registry entry plus its current size on disk.
//!
//! The size is not stored in the registry; it is read from the file system whenever the map
//! list leaves the daemon (HTTP API and backend sync), so it is always current and needs no
//! migration of existing registries.

use std::ops::Deref;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::registry::models::MapEntry;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapListing {
    #[serde(flatten)]
    pub entry: MapEntry,
    /// Size of the installed file (or directory, recursively) in bytes; `None` when it is
    /// missing or unreadable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

impl Deref for MapListing {
    type Target = MapEntry;

    fn deref(&self) -> &MapEntry {
        &self.entry
    }
}

impl From<MapEntry> for MapListing {
    /// A listing without size information.
    fn from(entry: MapEntry) -> Self {
        Self {
            entry,
            size_bytes: None,
        }
    }
}

/// Size of `installed_path` (relative to `addons_dir`) on disk. Directories are summed
/// recursively (symlinks are not followed). Paths that are absolute or contain `..` are
/// rejected.
pub fn disk_size(addons_dir: &Path, installed_path: &str) -> Option<u64> {
    let relative = Path::new(installed_path);
    if installed_path.trim().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    path_size(&addons_dir.join(relative))
}

fn path_size(path: &Path) -> Option<u64> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.is_file() {
        return Some(metadata.len());
    }
    if !metadata.is_dir() {
        return None;
    }
    let mut total = 0u64;
    for entry in std::fs::read_dir(path).ok()?.flatten() {
        total = total.saturating_add(path_size(&entry.path()).unwrap_or(0));
    }
    Some(total)
}

/// Attaches the current disk size to each entry (file system work runs off the async runtime).
pub async fn with_disk_sizes(entries: Vec<MapEntry>, addons_dir: PathBuf) -> Vec<MapListing> {
    let fallback = entries.clone();
    tokio::task::spawn_blocking(move || {
        entries
            .into_iter()
            .map(|entry| MapListing {
                size_bytes: disk_size(&addons_dir, &entry.installed_path),
                entry,
            })
            .collect()
    })
    .await
    .unwrap_or_else(|_| fallback.into_iter().map(MapListing::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(installed_path: &str) -> MapEntry {
        MapEntry::new(
            1,
            "Test".to_string(),
            "https://example.com/map.zip".to_string(),
            installed_path.to_string(),
        )
    }

    #[test]
    fn disk_size_of_file_and_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("map.vpk"), vec![0u8; 1234]).unwrap();
        std::fs::create_dir_all(dir.path().join("campaign/sub")).unwrap();
        std::fs::write(dir.path().join("campaign/a.vpk"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.path().join("campaign/sub/b.vpk"), vec![0u8; 50]).unwrap();

        assert_eq!(disk_size(dir.path(), "map.vpk"), Some(1234));
        assert_eq!(disk_size(dir.path(), "campaign"), Some(150));
        assert_eq!(disk_size(dir.path(), "missing.vpk"), None);
    }

    #[test]
    fn disk_size_rejects_paths_outside_addons() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(disk_size(dir.path(), "../etc/passwd"), None);
        assert_eq!(disk_size(dir.path(), "/etc/passwd"), None);
        assert_eq!(disk_size(dir.path(), ""), None);
    }

    #[tokio::test]
    async fn listing_serializes_flat_with_size() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("map.vpk"), vec![0u8; 42]).unwrap();
        let listings = with_disk_sizes(
            vec![entry("map.vpk"), entry("gone.vpk")],
            dir.path().to_path_buf(),
        )
        .await;

        let json = serde_json::to_value(&listings).unwrap();
        assert_eq!(json[0]["installed_path"], "map.vpk");
        assert_eq!(json[0]["size_bytes"], 42);
        assert!(json[1].get("size_bytes").is_none());

        // Older consumers that only know `MapEntry` still parse the payload.
        let parsed: Vec<MapEntry> = serde_json::from_value(json).unwrap();
        assert_eq!(parsed[0].installed_path, "map.vpk");
        assert_eq!(listings[0].id, 1); // Deref to MapEntry
    }
}
