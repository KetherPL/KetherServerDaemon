// SPDX-License-Identifier: GPL-3.0-only
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapEntry {
    /// Unique identifier for the map
    pub id: String,
    
    /// Display name of the map
    pub name: String,
    
    /// Original download URL
    pub source_url: String,
    
    /// Steam Workshop ID if applicable
    pub workshop_id: Option<u64>,
    
    /// Local installation path
    pub installed_path: PathBuf,
    
    /// Installation timestamp
    pub installed_at: DateTime<Utc>,
    
    /// Map version if available
    pub version: Option<String>,
}

impl MapEntry {
    pub fn new(
        id: String,
        name: String,
        source_url: String,
        installed_path: PathBuf,
    ) -> Self {
        Self {
            id,
            name,
            source_url,
            workshop_id: None,
            installed_path,
            installed_at: Utc::now(),
            version: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_map_entry_new() {
        let entry = MapEntry::new(
            "test-id".to_string(),
            "Test Map".to_string(),
            "https://example.com/map.zip".to_string(),
            PathBuf::from("/path/to/map"),
        );

        assert_eq!(entry.id, "test-id");
        assert_eq!(entry.name, "Test Map");
        assert_eq!(entry.source_url, "https://example.com/map.zip");
        assert_eq!(entry.installed_path, PathBuf::from("/path/to/map"));
        assert_eq!(entry.workshop_id, None);
        assert_eq!(entry.version, None);
        assert!(entry.installed_at <= Utc::now());
    }

    #[test]
    fn test_map_entry_serialize_json() {
        let entry = MapEntry {
            id: "test-id".to_string(),
            name: "Test Map".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            workshop_id: Some(123456789),
            installed_path: PathBuf::from("/path/to/map"),
            installed_at: Utc::now(),
            version: Some("1.0.0".to_string()),
        };

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"id\":\"test-id\""));
        assert!(json.contains("\"name\":\"Test Map\""));
        assert!(json.contains("\"source_url\":\"https://example.com/map.zip\""));
        assert!(json.contains("\"workshop_id\":123456789"));
        assert!(json.contains("\"version\":\"1.0.0\""));
    }

    #[test]
    fn test_map_entry_deserialize_json() {
        let json = r#"{
            "id": "test-id",
            "name": "Test Map",
            "source_url": "https://example.com/map.zip",
            "workshop_id": 123456789,
            "installed_path": "/path/to/map",
            "installed_at": "2024-01-01T00:00:00Z",
            "version": "1.0.0"
        }"#;

        let entry: MapEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.id, "test-id");
        assert_eq!(entry.name, "Test Map");
        assert_eq!(entry.source_url, "https://example.com/map.zip");
        assert_eq!(entry.workshop_id, Some(123456789));
        assert_eq!(entry.installed_path, PathBuf::from("/path/to/map"));
        assert_eq!(entry.version, Some("1.0.0".to_string()));
    }

    #[test]
    fn test_map_entry_serialize_deserialize_roundtrip() {
        let original = MapEntry {
            id: "test-id".to_string(),
            name: "Test Map".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            workshop_id: Some(123456789),
            installed_path: PathBuf::from("/path/to/map"),
            installed_at: Utc::now(),
            version: Some("1.0.0".to_string()),
        };

        let json = serde_json::to_string(&original).unwrap();
        let deserialized: MapEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(original.id, deserialized.id);
        assert_eq!(original.name, deserialized.name);
        assert_eq!(original.source_url, deserialized.source_url);
        assert_eq!(original.workshop_id, deserialized.workshop_id);
        assert_eq!(original.installed_path, deserialized.installed_path);
        assert_eq!(original.version, deserialized.version);
    }

    #[test]
    fn test_map_entry_with_optional_fields_none() {
        let entry = MapEntry {
            id: "test-id".to_string(),
            name: "Test Map".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            workshop_id: None,
            installed_path: PathBuf::from("/path/to/map"),
            installed_at: Utc::now(),
            version: None,
        };

        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: MapEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.workshop_id, None);
        assert_eq!(deserialized.version, None);
    }
}

