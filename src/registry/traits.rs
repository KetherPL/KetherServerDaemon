// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use crate::registry::models::MapEntry;

#[async_trait]
pub trait Registry: Send + Sync {
    /// Add a new map entry to the registry
    /// Returns the assigned auto-increment ID
    async fn add_map(&self, entry: MapEntry) -> anyhow::Result<u64>;
    
    /// Remove a map entry from the registry
    async fn remove_map(&self, id: u64) -> anyhow::Result<()>;
    
    /// Get a map entry by ID
    async fn get_map(&self, id: u64) -> anyhow::Result<Option<MapEntry>>;
    
    /// List all map entries
    async fn list_maps(&self) -> anyhow::Result<Vec<MapEntry>>;
    
    /// Update an existing map entry
    async fn update_map(&self, entry: MapEntry) -> anyhow::Result<()>;
}

