// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use crate::registry::models::MapEntry;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapUpdate {
    pub action: String, // "install", "uninstall", "update"
    pub map_id: String,
    pub map_entry: Option<MapEntry>,
}

#[async_trait]
pub trait SyncService: Send + Sync {
    /// Push local registry state to backend
    async fn sync_registry(&self, entries: Vec<MapEntry>) -> anyhow::Result<()>;
    
    /// Fetch pending updates from backend
    async fn fetch_updates(&self) -> anyhow::Result<Vec<MapUpdate>>;
}

