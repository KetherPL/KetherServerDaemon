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

