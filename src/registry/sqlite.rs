// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use sqlx::{sqlite::SqlitePool, Row};
use std::path::PathBuf;
use crate::registry::{models::{MapEntry, SourceKind}, traits::Registry};
use tracing::{error, info, warn};

pub struct SqliteRegistry {
    pool: SqlitePool,
}

impl SqliteRegistry {
    pub async fn new(db_path: &PathBuf) -> anyhow::Result<Self> {
        let db_url = format!("sqlite:{}", db_path.display());
        let pool = SqlitePool::connect(&db_url).await?;
        
        let registry = Self { pool };
        registry.init_schema().await?;
        
        Ok(registry)
    }
    
    async fn init_schema(&self) -> anyhow::Result<()> {
        // Create table if it doesn't exist
        // Note: This will fail on existing databases with TEXT id - that's expected (fresh database required)
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS maps (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                source_url TEXT NOT NULL,
                workshop_id INTEGER,
                installed_path TEXT NOT NULL,
                installed_at TEXT NOT NULL,
                version TEXT,
                source_kind TEXT NOT NULL DEFAULT 'other',
                checksum TEXT,
                checksum_kind TEXT
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        
        info!("Initialized SQLite registry schema");
        Ok(())
    }
    
    
    fn map_entry_from_row(&self, row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<MapEntry> {
        use chrono::DateTime;
        
        let source_kind_str: String = row.try_get::<String, _>("source_kind")
            .unwrap_or_else(|_| "other".to_string()); // Default to "other" for old records
        let source_kind = match source_kind_str.as_str() {
            "workshop" => SourceKind::Workshop,
            "sirplease" => SourceKind::SirPlease,
            "other" => SourceKind::Other,
            _ => {
                warn!(source_kind = %source_kind_str, "Unknown source_kind, defaulting to 'other'");
                SourceKind::Other
            }
        };
        
        let workshop_id = row.get::<Option<i64>, _>("workshop_id").map(|v| v as u64);
        // Ensure workshop_id is None if source_kind is not Workshop
        let workshop_id = if source_kind == SourceKind::Workshop {
            workshop_id
        } else {
            None
        };
        
        Ok(MapEntry {
            id: row.get::<i64, _>("id") as u64,
            name: row.get::<String, _>("name"),
            source_url: row.get::<String, _>("source_url"),
            source_kind,
            workshop_id,
            installed_path: row.get::<String, _>("installed_path"), // Now stored as String
            installed_at: DateTime::parse_from_rfc3339(&row.get::<String, _>("installed_at"))?
                .with_timezone(&chrono::Utc),
            version: row.get::<Option<String>, _>("version"),
            checksum: row.try_get("checksum").ok(),
            checksum_kind: row.try_get("checksum_kind").ok(),
        })
    }
}

#[async_trait]
impl Registry for SqliteRegistry {
    async fn add_map(&self, entry: MapEntry) -> anyhow::Result<u64> {
        let source_kind_str = match entry.source_kind {
            SourceKind::Workshop => "workshop",
            SourceKind::SirPlease => "sirplease",
            SourceKind::Other => "other",
        };
        
        // Ensure workshop_id is None if source_kind is not Workshop
        let workshop_id = if entry.source_kind == SourceKind::Workshop {
            entry.workshop_id.map(|v| v as i64)
        } else {
            None
        };
        
        // Insert without ID (database will assign auto-increment ID)
        let result = sqlx::query(
            r#"
            INSERT INTO maps (name, source_url, source_kind, workshop_id, installed_path, installed_at, version, checksum, checksum_kind)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(&entry.name)
        .bind(&entry.source_url)
        .bind(source_kind_str)
        .bind(workshop_id)
        .bind(&entry.installed_path)
        .bind(entry.installed_at.to_rfc3339())
        .bind(&entry.version)
        .bind(&entry.checksum)
        .bind(&entry.checksum_kind)
        .execute(&self.pool)
        .await?;
        
        // Get the assigned ID from last_insert_rowid
        let assigned_id = result.last_insert_rowid() as u64;
        
        info!(map_id = assigned_id, "Added map to registry");
        Ok(assigned_id)
    }
    
    async fn remove_map(&self, id: u64) -> anyhow::Result<()> {
        let result = sqlx::query("DELETE FROM maps WHERE id = ?1")
            .bind(id as i64)
            .execute(&self.pool)
            .await?;
        
        if result.rows_affected() > 0 {
            info!(map_id = id, "Removed map from registry");
        }
        
        Ok(())
    }
    
    async fn get_map(&self, id: u64) -> anyhow::Result<Option<MapEntry>> {
        let row = sqlx::query("SELECT * FROM maps WHERE id = ?1")
            .bind(id as i64)
            .fetch_optional(&self.pool)
            .await?;
        
        match row {
            Some(row) => Ok(Some(self.map_entry_from_row(&row)?)),
            None => Ok(None),
        }
    }
    
    async fn list_maps(&self) -> anyhow::Result<Vec<MapEntry>> {
        let rows = sqlx::query("SELECT * FROM maps")
            .fetch_all(&self.pool)
            .await?;
        
        let mut entries = Vec::new();
        for row in rows {
            match self.map_entry_from_row(&row) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    error!(error = %e, "Failed to parse map entry from database");
                }
            }
        }
        
        Ok(entries)
    }
    
    async fn update_map(&self, entry: MapEntry) -> anyhow::Result<()> {
        let source_kind_str = match entry.source_kind {
            SourceKind::Workshop => "workshop",
            SourceKind::SirPlease => "sirplease",
            SourceKind::Other => "other",
        };
        
        // Ensure workshop_id is None if source_kind is not Workshop
        let workshop_id = if entry.source_kind == SourceKind::Workshop {
            entry.workshop_id.map(|v| v as i64)
        } else {
            None
        };
        
        sqlx::query(
            r#"
            UPDATE maps
            SET name = ?2, source_url = ?3, source_kind = ?4, workshop_id = ?5, installed_path = ?6, installed_at = ?7, version = ?8, checksum = ?9, checksum_kind = ?10
            WHERE id = ?1
            "#,
        )
        .bind(entry.id as i64)
        .bind(&entry.name)
        .bind(&entry.source_url)
        .bind(source_kind_str)
        .bind(workshop_id)
        .bind(&entry.installed_path)
        .bind(entry.installed_at.to_rfc3339())
        .bind(&entry.version)
        .bind(&entry.checksum)
        .bind(&entry.checksum_kind)
        .execute(&self.pool)
        .await?;
        
        info!(map_id = entry.id, "Updated map in registry");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
use super::*;
use crate::registry::models;
use tempfile::NamedTempFile;
use chrono::Utc;

    async fn setup_test_registry() -> SqliteRegistry {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path().to_path_buf();
        SqliteRegistry::new(&db_path).await.unwrap()
    }

    fn create_test_map_entry(id: u64) -> MapEntry {
        MapEntry {
            id,
            name: "Test Map".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            source_kind: models::SourceKind::Workshop,
            workshop_id: Some(123456789),
            installed_path: "test_map.vpk".to_string(),
            installed_at: Utc::now(),
            version: Some("1.0.0".to_string()),
            checksum: Some("abc123def456".to_string()),
            checksum_kind: Some("md5".to_string()),
        }
    }

    #[tokio::test]
    async fn test_schema_initialization() {
        let registry = setup_test_registry().await;
        // Schema should be initialized on creation, test by trying to query
        let maps = registry.list_maps().await.unwrap();
        assert_eq!(maps.len(), 0);
    }

    #[tokio::test]
    async fn test_add_map() {
        let registry = setup_test_registry().await;
        let mut entry = create_test_map_entry(0); // ID will be assigned by database
        
        let assigned_id = registry.add_map(entry.clone()).await.unwrap();
        entry.id = assigned_id;
        
        let retrieved = registry.get_map(assigned_id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved_entry = retrieved.unwrap();
        assert_eq!(retrieved_entry.id, entry.id);
        assert_eq!(retrieved_entry.name, entry.name);
        assert_eq!(retrieved_entry.source_url, entry.source_url);
        assert_eq!(retrieved_entry.workshop_id, entry.workshop_id);
    }

    #[tokio::test]
    async fn test_add_map_without_optional_fields() {
        let registry = setup_test_registry().await;
        let mut entry = MapEntry {
            id: 0, // Will be assigned by database
            name: "Test Map 2".to_string(),
            source_url: "https://example.com/map2.zip".to_string(),
            source_kind: models::SourceKind::Other,
            workshop_id: None,
            installed_path: "test_map2.vpk".to_string(),
            installed_at: Utc::now(),
            version: None,
            checksum: None,
            checksum_kind: None,
        };
        
        let assigned_id = registry.add_map(entry.clone()).await.unwrap();
        entry.id = assigned_id;
        
        let retrieved = registry.get_map(assigned_id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved_entry = retrieved.unwrap();
        assert_eq!(retrieved_entry.workshop_id, None);
        assert_eq!(retrieved_entry.version, None);
    }

    #[tokio::test]
    async fn test_get_map_exists() {
        let registry = setup_test_registry().await;
        let mut entry = create_test_map_entry(0);
        let assigned_id = registry.add_map(entry.clone()).await.unwrap();
        entry.id = assigned_id;
        
        let retrieved = registry.get_map(assigned_id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, assigned_id);
    }

    #[tokio::test]
    async fn test_get_map_not_exists() {
        let registry = setup_test_registry().await;
        
        let retrieved = registry.get_map(99999).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_remove_map() {
        let registry = setup_test_registry().await;
        let mut entry = create_test_map_entry(0);
        let assigned_id = registry.add_map(entry).await.unwrap();
        
        // Verify it exists
        assert!(registry.get_map(assigned_id).await.unwrap().is_some());
        
        // Remove it
        registry.remove_map(assigned_id).await.unwrap();
        
        // Verify it's gone
        assert!(registry.get_map(assigned_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_remove_map_not_exists() {
        let registry = setup_test_registry().await;
        
        // Should not error when removing non-existent map
        registry.remove_map(99999).await.unwrap();
    }

    #[tokio::test]
    async fn test_list_maps_empty() {
        let registry = setup_test_registry().await;
        let maps = registry.list_maps().await.unwrap();
        assert_eq!(maps.len(), 0);
    }

    #[tokio::test]
    async fn test_list_maps_multiple() {
        let registry = setup_test_registry().await;
        let mut entry1 = create_test_map_entry(0);
        let mut entry2 = create_test_map_entry(0);
        let mut entry3 = MapEntry {
            id: 0, // Will be assigned by database
            name: "Test Map 7".to_string(),
            source_url: "https://example.com/map7.zip".to_string(),
            source_kind: models::SourceKind::Other,
            workshop_id: None,
            installed_path: "test_map7.vpk".to_string(),
            installed_at: Utc::now(),
            version: None,
            checksum: None,
            checksum_kind: None,
        };
        
        let id1 = registry.add_map(entry1).await.unwrap();
        let id2 = registry.add_map(entry2).await.unwrap();
        let id3 = registry.add_map(entry3).await.unwrap();
        
        let maps = registry.list_maps().await.unwrap();
        assert_eq!(maps.len(), 3);
        
        let ids: Vec<u64> = maps.iter().map(|m| m.id).collect();
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
        assert!(ids.contains(&id3));
    }

    #[tokio::test]
    async fn test_update_map() {
        let registry = setup_test_registry().await;
        let mut entry = create_test_map_entry(0);
        let assigned_id = registry.add_map(entry.clone()).await.unwrap();
        entry.id = assigned_id;
        
        // Update the entry
        entry.name = "Updated Map Name".to_string();
        entry.version = Some("2.0.0".to_string());
        registry.update_map(entry.clone()).await.unwrap();
        
        let retrieved = registry.get_map(assigned_id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved_entry = retrieved.unwrap();
        assert_eq!(retrieved_entry.name, "Updated Map Name");
        assert_eq!(retrieved_entry.version, Some("2.0.0".to_string()));
    }

    #[tokio::test]
    async fn test_auto_increment_ids() {
        let registry = setup_test_registry().await;
        let mut entry1 = create_test_map_entry(0);
        let mut entry2 = create_test_map_entry(0);
        
        let id1 = registry.add_map(entry1.clone()).await.unwrap();
        let id2 = registry.add_map(entry2.clone()).await.unwrap();
        
        // IDs should be sequential
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }

    #[tokio::test]
    async fn test_update_non_existent_map() {
        let registry = setup_test_registry().await;
        let mut entry = create_test_map_entry(0);
        entry.id = 99999; // Non-existent ID
        
        // Update should not error even if map doesn't exist
        // (it will just update 0 rows)
        registry.update_map(entry).await.unwrap();
    }

    #[tokio::test]
    async fn test_map_with_long_path() {
        let registry = setup_test_registry().await;
        let long_path = "/".to_string() + &"a".repeat(500) + "/test/map";
        let mut entry = MapEntry {
            id: 0, // Will be assigned by database
            name: "Test Map".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            source_kind: models::SourceKind::Other,
            workshop_id: None,
            installed_path: long_path.clone(),
            installed_at: Utc::now(),
            version: None,
            checksum: None,
            checksum_kind: None,
        };
        
        let assigned_id = registry.add_map(entry.clone()).await.unwrap();
        entry.id = assigned_id;
        
        let retrieved = registry.get_map(assigned_id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().installed_path, long_path);
    }
}

