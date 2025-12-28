// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use sqlx::{sqlite::SqlitePool, Row};
use std::path::PathBuf;
use crate::registry::{models::MapEntry, traits::Registry};
use tracing::{error, info};

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
        
        info!("Initialized SQLite registry schema");
        Ok(())
    }
    
    fn map_entry_from_row(&self, row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<MapEntry> {
        use chrono::DateTime;
        
        Ok(MapEntry {
            id: row.get::<String, _>("id"),
            name: row.get::<String, _>("name"),
            source_url: row.get::<String, _>("source_url"),
            workshop_id: row.get::<Option<i64>, _>("workshop_id").map(|v| v as u64),
            installed_path: PathBuf::from(row.get::<String, _>("installed_path")),
            installed_at: DateTime::parse_from_rfc3339(&row.get::<String, _>("installed_at"))?
                .with_timezone(&chrono::Utc),
            version: row.get::<Option<String>, _>("version"),
        })
    }
}

#[async_trait]
impl Registry for SqliteRegistry {
    async fn add_map(&self, entry: MapEntry) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO maps (id, name, source_url, workshop_id, installed_path, installed_at, version)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(&entry.id)
        .bind(&entry.name)
        .bind(&entry.source_url)
        .bind(entry.workshop_id.map(|v| v as i64))
        .bind(entry.installed_path.to_string_lossy().to_string())
        .bind(entry.installed_at.to_rfc3339())
        .bind(&entry.version)
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
        sqlx::query(
            r#"
            UPDATE maps
            SET name = ?2, source_url = ?3, workshop_id = ?4, installed_path = ?5, installed_at = ?6, version = ?7
            WHERE id = ?1
            "#,
        )
        .bind(&entry.id)
        .bind(&entry.name)
        .bind(&entry.source_url)
        .bind(entry.workshop_id.map(|v| v as i64))
        .bind(entry.installed_path.to_string_lossy().to_string())
        .bind(entry.installed_at.to_rfc3339())
        .bind(&entry.version)
        .execute(&self.pool)
        .await?;
        
        info!(map_id = %entry.id, "Updated map in registry");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            workshop_id: Some(123456789),
            installed_path: PathBuf::from("/test/path"),
            installed_at: Utc::now(),
            version: Some("1.0.0".to_string()),
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
            workshop_id: None,
            installed_path: PathBuf::from("/test/path2"),
            installed_at: Utc::now(),
            version: None,
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
            workshop_id: None,
            installed_path: PathBuf::from("/test/path7"),
            installed_at: Utc::now(),
            version: None,
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
            workshop_id: None,
            installed_path: PathBuf::from(&long_path),
            installed_at: Utc::now(),
            version: None,
        };
        
        registry.add_map(entry.clone()).await.unwrap();
        
        let retrieved = registry.get_map("test-id-11").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().installed_path, PathBuf::from(&long_path));
    }
}

