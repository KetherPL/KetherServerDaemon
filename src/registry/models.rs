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

