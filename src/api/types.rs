// SPDX-License-Identifier: GPL-3.0-only
use serde::{Deserialize, Serialize};

use crate::map_installer::DiscoveryMode;

#[derive(Debug, Serialize, Deserialize)]
pub struct InstallMapRequest {
    /// HTTP/HTTPS URL for ZIP file download (only used when workshop_id is not provided)
    pub url: Option<String>,

    /// Steam Workshop ID (only used when url is not provided)
    pub workshop_id: Option<u64>,

    /// Optional map name override
    pub name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateWorkshopRequest {
    pub map_id: Option<u64>,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub check_only: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiscoverRequest {
    #[serde(default)]
    pub mode: DiscoveryMode,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModifyMapRequest {
    pub field: String,
    pub value: String,
}
