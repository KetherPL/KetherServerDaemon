// SPDX-License-Identifier: GPL-3.0-only

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The systemd user service name that runs the L4D2 server.
pub const DEFAULT_L4D2_SERVICE: &str = "l4d2-srcds1.service";

/// Active state of the systemd unit, as reported by `systemctl --user is-active`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitActiveState {
    Active,
    Inactive,
    Activating,
    Deactivating,
    Failed,
    Unknown,
}

impl fmt::Display for UnitActiveState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Active => "active",
            Self::Inactive => "inactive",
            Self::Activating => "activating",
            Self::Deactivating => "deactivating",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        };
        write!(f, "{s}")
    }
}

impl From<&str> for UnitActiveState {
    fn from(s: &str) -> Self {
        match s.trim() {
            "active" => Self::Active,
            "inactive" => Self::Inactive,
            "activating" => Self::Activating,
            "deactivating" => Self::Deactivating,
            "failed" => Self::Failed,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameServerStatus {
    /// Systemd unit name queried.
    pub service: String,
    /// Parsed active state from `systemctl --user is-active`.
    pub active_state: UnitActiveState,
    /// Raw string from `systemctl --user is-active` (may carry extra info).
    pub active_state_raw: String,
    /// Sub-state from `systemctl --user show --property=SubState`.
    pub sub_state: String,
    /// Description from `systemctl --user show --property=Description`.
    pub description: String,
    /// Main PID (0 when not running), from `systemctl --user show --property=MainPID`.
    pub main_pid: u32,
    /// Process name for main_pid (e.g. "SCREEN").
    pub main_comm: Option<String>,
    /// When the main service process started.
    pub main_started_at: Option<DateTime<Utc>>,
    /// Uptime in seconds of the main service process.
    pub main_uptime_secs: Option<u64>,
    /// SRCDS process PID if found.
    pub srcds_pid: Option<u32>,
    /// Process name for srcds_pid (e.g. "srcds_linux").
    pub srcds_comm: Option<String>,
    /// When the SRCDS process started.
    pub srcds_started_at: Option<DateTime<Utc>>,
    /// Uptime in seconds of the SRCDS process.
    pub srcds_uptime_secs: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
}

impl fmt::Display for ServiceAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start => write!(f, "start"),
            Self::Stop => write!(f, "stop"),
            Self::Restart => write!(f, "restart"),
        }
    }
}
