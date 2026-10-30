// SPDX-License-Identifier: GPL-3.0-only
//! Control the L4D2 game server via systemctl --user.

use std::fmt;
use std::process::Command;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

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
}

/// Build the environment required for `systemctl --user` to work when the
/// process is not running inside a login session (e.g. inside another systemd
/// user service that doesn't inherit the D-Bus socket).
fn systemctl_user_env(uid: u32) -> Vec<(String, String)> {
    vec![
        (
            "XDG_RUNTIME_DIR".to_string(),
            format!("/run/user/{uid}"),
        ),
        (
            "DBUS_SESSION_BUS_ADDRESS".to_string(),
            format!("unix:path=/run/user/{uid}/bus"),
        ),
    ]
}

fn systemctl_user(uid: u32, args: &[&str]) -> std::io::Result<std::process::Output> {
    let mut cmd = Command::new("systemctl");
    cmd.arg("--user");
    for arg in args {
        cmd.arg(arg);
    }
    for (k, v) in systemctl_user_env(uid) {
        cmd.env(k, v);
    }
    cmd.output()
}

/// Read a single property from `systemctl --user show`.
fn show_property(uid: u32, service: &str, property: &str) -> String {
    let output = systemctl_user(uid, &["show", service, &format!("--property={property}")]);
    let stdout = match output {
        Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
        Err(_) => String::new(),
    };
    // Output format: "Property=value\n"
    for line in stdout.lines() {
        if let Some(val) = line.strip_prefix(&format!("{property}=")) {
            return val.trim().to_string();
        }
    }
    String::new()
}

/// Query the current status of the game server service.
pub fn game_server_status(uid: u32, service: &str) -> GameServerStatus {
    let active_state_raw = match systemctl_user(uid, &["is-active", service]) {
        Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Err(_) => "unknown".to_string(),
    };
    let active_state = UnitActiveState::from(active_state_raw.as_str());

    let sub_state = show_property(uid, service, "SubState");
    let description = show_property(uid, service, "Description");
    let main_pid_str = show_property(uid, service, "MainPID");
    let main_pid = main_pid_str.parse::<u32>().unwrap_or(0);

    GameServerStatus {
        service: service.to_string(),
        active_state,
        active_state_raw,
        sub_state,
        description,
        main_pid,
    }
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

/// Run `systemctl --user <action> <service>`.
/// Returns Ok(()) on success (exit code 0), Err with description on failure.
pub fn game_server_action(uid: u32, service: &str, action: ServiceAction) -> Result<(), String> {
    let action_str = action.to_string();
    info!(uid, service, action = %action, "Issuing game server systemctl action");
    match systemctl_user(uid, &[&action_str, service]) {
        Ok(output) if output.status.success() => {
            info!(uid, service, action = %action, "Game server systemctl action succeeded");
            Ok(())
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let msg = format!(
                "systemctl --user {action} {service} failed (exit {}): {stderr} {stdout}",
                output.status
            );
            warn!("{msg}");
            Err(msg)
        }
        Err(e) => {
            let msg = format!("Failed to run systemctl: {e}");
            warn!("{msg}");
            Err(msg)
        }
    }
}
