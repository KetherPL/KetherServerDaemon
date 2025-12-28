// SPDX-License-Identifier: GPL-3.0-only
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Workshop,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapEntry {
    /// Unique identifier for the map
    pub id: String,
    
    /// Display name of the map
    pub name: String,
    
    /// Original download URL
    pub source_url: String,
    
    /// Source kind: workshop or other
    pub source_kind: SourceKind,
    
    /// Steam Workshop ID (only present when source_kind is Workshop)
    pub workshop_id: Option<u64>,
    
    /// Local installation path (relative to addons/ directory)
    pub installed_path: String,
    
    /// Installation timestamp
    pub installed_at: DateTime<Utc>,
    
    /// Map version if available
    pub version: Option<String>,
    
    /// File checksum (MD5 hex string)
    pub checksum: Option<String>,
    
    /// Checksum algorithm kind (currently only "md5")
    pub checksum_kind: Option<String>,
}

impl MapEntry {
    pub fn new(
        id: String,
        name: String,
        source_url: String,
        installed_path: String,
    ) -> Self {
        Self {
            id,
            name,
            source_url,
            source_kind: SourceKind::Other,
            workshop_id: None,
            installed_path,
            installed_at: Utc::now(),
            version: None,
            checksum: None,
            checksum_kind: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_entry_new() {
        let entry = MapEntry::new(
            "test-id".to_string(),
            "Test Map".to_string(),
            "https://example.com/map.zip".to_string(),
            "map.vpk".to_string(),
        );

        assert_eq!(entry.id, "test-id");
        assert_eq!(entry.name, "Test Map");
        assert_eq!(entry.source_url, "https://example.com/map.zip");
        assert_eq!(entry.installed_path, "map.vpk");
        assert_eq!(entry.source_kind, SourceKind::Other);
        assert_eq!(entry.workshop_id, None);
        assert_eq!(entry.version, None);
        assert_eq!(entry.checksum, None);
        assert_eq!(entry.checksum_kind, None);
        assert!(entry.installed_at <= Utc::now());
    }

    #[test]
    fn test_map_entry_serialize_json() {
        let entry = MapEntry {
            id: "test-id".to_string(),
            name: "Test Map".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            source_kind: SourceKind::Workshop,
            workshop_id: Some(123456789),
            installed_path: "workshop/map.vpk".to_string(),
            installed_at: Utc::now(),
            version: Some("1.0.0".to_string()),
            checksum: Some("abc123def456".to_string()),
            checksum_kind: Some("md5".to_string()),
        };

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"id\":\"test-id\""));
        assert!(json.contains("\"name\":\"Test Map\""));
        assert!(json.contains("\"source_url\":\"https://example.com/map.zip\""));
        assert!(json.contains("\"source_kind\":\"workshop\""));
        assert!(json.contains("\"workshop_id\":123456789"));
        assert!(json.contains("\"installed_path\":\"workshop/map.vpk\""));
        assert!(json.contains("\"version\":\"1.0.0\""));
        assert!(json.contains("\"checksum\":\"abc123def456\""));
        assert!(json.contains("\"checksum_kind\":\"md5\""));
    }

    #[test]
    fn test_map_entry_deserialize_json() {
        let json = r#"{
            "id": "test-id",
            "name": "Test Map",
            "source_url": "https://example.com/map.zip",
            "source_kind": "workshop",
            "workshop_id": 123456789,
            "installed_path": "map.vpk",
            "installed_at": "2024-01-01T00:00:00Z",
            "version": "1.0.0",
            "checksum": "abc123def456",
            "checksum_kind": "md5"
        }"#;

        let entry: MapEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.id, "test-id");
        assert_eq!(entry.name, "Test Map");
        assert_eq!(entry.source_url, "https://example.com/map.zip");
        assert_eq!(entry.source_kind, SourceKind::Workshop);
        assert_eq!(entry.workshop_id, Some(123456789));
        assert_eq!(entry.installed_path, "map.vpk");
        assert_eq!(entry.version, Some("1.0.0".to_string()));
        assert_eq!(entry.checksum, Some("abc123def456".to_string()));
        assert_eq!(entry.checksum_kind, Some("md5".to_string()));
    }

    #[test]
    fn test_map_entry_serialize_deserialize_roundtrip() {
        let original = MapEntry {
            id: "test-id".to_string(),
            name: "Test Map".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            source_kind: SourceKind::Other,
            workshop_id: None,
            installed_path: "map.vpk".to_string(),
            installed_at: Utc::now(),
            version: Some("1.0.0".to_string()),
            checksum: Some("abc123def456".to_string()),
            checksum_kind: Some("md5".to_string()),
        };

        let json = serde_json::to_string(&original).unwrap();
        let deserialized: MapEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(original.id, deserialized.id);
        assert_eq!(original.name, deserialized.name);
        assert_eq!(original.source_url, deserialized.source_url);
        assert_eq!(original.source_kind, deserialized.source_kind);
        assert_eq!(original.workshop_id, deserialized.workshop_id);
        assert_eq!(original.installed_path, deserialized.installed_path);
        assert_eq!(original.version, deserialized.version);
        assert_eq!(original.checksum, deserialized.checksum);
        assert_eq!(original.checksum_kind, deserialized.checksum_kind);
    }

    #[test]
    fn test_map_entry_with_optional_fields_none() {
        let entry = MapEntry {
            id: "test-id".to_string(),
            name: "Test Map".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            source_kind: SourceKind::Other,
            workshop_id: None,
            installed_path: "map.vpk".to_string(),
            installed_at: Utc::now(),
            version: None,
            checksum: None,
            checksum_kind: None,
        };

        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: MapEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.source_kind, SourceKind::Other);
        assert_eq!(deserialized.workshop_id, None);
        assert_eq!(deserialized.version, None);
        assert_eq!(deserialized.checksum, None);
        assert_eq!(deserialized.checksum_kind, None);
    }
}

