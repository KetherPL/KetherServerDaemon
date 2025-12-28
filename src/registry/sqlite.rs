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
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS maps (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                source_url TEXT NOT NULL,
                workshop_id INTEGER,
                installed_path TEXT NOT NULL,
                installed_at TEXT NOT NULL,
                version TEXT
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        
        // Add new columns if they don't exist (migration)
        self.migrate_schema().await?;
        
        info!("Initialized SQLite registry schema");
        Ok(())
    }
    
    async fn migrate_schema(&self) -> anyhow::Result<()> {
        // Check if source_kind column exists by trying to query it
        let result = sqlx::query("SELECT source_kind FROM maps LIMIT 1")
            .fetch_optional(&self.pool)
            .await;
        
        if result.is_err() {
            // Column doesn't exist, need to add it
            info!("Migrating database schema: adding new columns");
            
            // Add source_kind column with default value
            sqlx::query(
                r#"
                ALTER TABLE maps ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'other'
                "#,
            )
            .execute(&self.pool)
            .await?;
            
            // Update existing records: if workshop_id is not null, set source_kind to 'workshop'
            sqlx::query(
                r#"
                UPDATE maps SET source_kind = 'workshop' WHERE workshop_id IS NOT NULL
                "#,
            )
            .execute(&self.pool)
            .await?;
            
            // Add checksum column (nullable)
            sqlx::query(
                r#"
                ALTER TABLE maps ADD COLUMN checksum TEXT
                "#,
            )
            .execute(&self.pool)
            .await?;
            
            // Add checksum_kind column (nullable)
            sqlx::query(
                r#"
                ALTER TABLE maps ADD COLUMN checksum_kind TEXT
                "#,
            )
            .execute(&self.pool)
            .await?;
            
            // Note: For existing records with absolute paths, we can't automatically
            // convert them to relative paths without knowing the addons_dir.
            // This will be handled at read time or require manual migration.
            warn!("Database migrated. Existing absolute paths in installed_path may need manual conversion to relative paths.");
        }
        
        Ok(())
    }
    
    fn map_entry_from_row(&self, row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<MapEntry> {
        use chrono::DateTime;
        
        let source_kind_str: String = row.try_get::<String, _>("source_kind")
            .unwrap_or_else(|_| "other".to_string()); // Default to "other" for old records
        let source_kind = match source_kind_str.as_str() {
            "workshop" => SourceKind::Workshop,
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
            id: row.get::<String, _>("id"),
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
    async fn add_map(&self, entry: MapEntry) -> anyhow::Result<()> {
        let source_kind_str = match entry.source_kind {
            SourceKind::Workshop => "workshop",
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
            INSERT INTO maps (id, name, source_url, source_kind, workshop_id, installed_path, installed_at, version, checksum, checksum_kind)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(&entry.id)
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
        
        info!(map_id = %entry.id, "Added map to registry");
        Ok(())
    }
    
    async fn remove_map(&self, id: &str) -> anyhow::Result<()> {
        let result = sqlx::query("DELETE FROM maps WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        
        if result.rows_affected() > 0 {
            info!(map_id = %id, "Removed map from registry");
        }
        
        Ok(())
    }
    
    async fn get_map(&self, id: &str) -> anyhow::Result<Option<MapEntry>> {
        let row = sqlx::query("SELECT * FROM maps WHERE id = ?1")
            .bind(id)
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
        .bind(&entry.id)
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
        
        info!(map_id = %entry.id, "Updated map in registry");
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

    fn create_test_map_entry(id: &str) -> MapEntry {
        MapEntry {
            id: id.to_string(),
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
        let entry = create_test_map_entry("test-id-1");
        
        registry.add_map(entry.clone()).await.unwrap();
        
        let retrieved = registry.get_map("test-id-1").await.unwrap();
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
        let entry = MapEntry {
            id: "test-id-2".to_string(),
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
        
        registry.add_map(entry.clone()).await.unwrap();
        
        let retrieved = registry.get_map("test-id-2").await.unwrap();
        assert!(retrieved.is_some());
        let retrieved_entry = retrieved.unwrap();
        assert_eq!(retrieved_entry.workshop_id, None);
        assert_eq!(retrieved_entry.version, None);
    }

    #[tokio::test]
    async fn test_get_map_exists() {
        let registry = setup_test_registry().await;
        let entry = create_test_map_entry("test-id-3");
        registry.add_map(entry.clone()).await.unwrap();
        
        let retrieved = registry.get_map("test-id-3").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, "test-id-3");
    }

    #[tokio::test]
    async fn test_get_map_not_exists() {
        let registry = setup_test_registry().await;
        
        let retrieved = registry.get_map("non-existent-id").await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_remove_map() {
        let registry = setup_test_registry().await;
        let entry = create_test_map_entry("test-id-4");
        registry.add_map(entry).await.unwrap();
        
        // Verify it exists
        assert!(registry.get_map("test-id-4").await.unwrap().is_some());
        
        // Remove it
        registry.remove_map("test-id-4").await.unwrap();
        
        // Verify it's gone
        assert!(registry.get_map("test-id-4").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_remove_map_not_exists() {
        let registry = setup_test_registry().await;
        
        // Should not error when removing non-existent map
        registry.remove_map("non-existent-id").await.unwrap();
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
        let entry1 = create_test_map_entry("test-id-5");
        let entry2 = create_test_map_entry("test-id-6");
        let entry3 = MapEntry {
            id: "test-id-7".to_string(),
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
        
        registry.add_map(entry1).await.unwrap();
        registry.add_map(entry2).await.unwrap();
        registry.add_map(entry3).await.unwrap();
        
        let maps = registry.list_maps().await.unwrap();
        assert_eq!(maps.len(), 3);
        
        let ids: Vec<String> = maps.iter().map(|m| m.id.clone()).collect();
        assert!(ids.contains(&"test-id-5".to_string()));
        assert!(ids.contains(&"test-id-6".to_string()));
        assert!(ids.contains(&"test-id-7".to_string()));
    }

    #[tokio::test]
    async fn test_update_map() {
        let registry = setup_test_registry().await;
        let mut entry = create_test_map_entry("test-id-8");
        registry.add_map(entry.clone()).await.unwrap();
        
        // Update the entry
        entry.name = "Updated Map Name".to_string();
        entry.version = Some("2.0.0".to_string());
        registry.update_map(entry.clone()).await.unwrap();
        
        let retrieved = registry.get_map("test-id-8").await.unwrap();
        assert!(retrieved.is_some());
        let retrieved_entry = retrieved.unwrap();
        assert_eq!(retrieved_entry.name, "Updated Map Name");
        assert_eq!(retrieved_entry.version, Some("2.0.0".to_string()));
    }

    #[tokio::test]
    async fn test_duplicate_id_error() {
        let registry = setup_test_registry().await;
        let entry1 = create_test_map_entry("test-id-9");
        let entry2 = create_test_map_entry("test-id-9");
        
        registry.add_map(entry1).await.unwrap();
        
        // SQLite will error on duplicate primary key
        let result = registry.add_map(entry2).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_non_existent_map() {
        let registry = setup_test_registry().await;
        let entry = create_test_map_entry("test-id-10");
        
        // Update should not error even if map doesn't exist
        // (it will just update 0 rows)
        registry.update_map(entry).await.unwrap();
    }

    #[tokio::test]
    async fn test_map_with_long_path() {
        let registry = setup_test_registry().await;
        let long_path = "/".to_string() + &"a".repeat(500) + "/test/map";
        let entry = MapEntry {
            id: "test-id-11".to_string(),
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
        
        registry.add_map(entry.clone()).await.unwrap();
        
        let retrieved = registry.get_map("test-id-11").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().installed_path, long_path);
    }
}

