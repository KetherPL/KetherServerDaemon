// SPDX-License-Identifier: GPL-3.0-only
//! Control the L4D2 game server via systemctl --user.

use std::collections::HashMap;
use std::fmt;
use std::process::Command;

use chrono::{DateTime, Utc};
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

/// Build the environment required for `systemctl --user` to work when the
/// process is not running inside a login session (e.g. inside another systemd
/// user service that doesn't inherit the D-Bus socket).
pub(crate) fn systemctl_user_env(uid: u32) -> Vec<(String, String)> {
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

/// Reads system uptime from /proc/uptime in seconds.
fn get_system_uptime() -> Option<f64> {
    let content = std::fs::read_to_string("/proc/uptime").ok()?;
    content.split_whitespace().next()?.parse::<f64>().ok()
}

/// Reads process stat information for a given pid: (comm, ppid, started_at, uptime_secs).
fn get_proc_stat(pid: u32, sys_uptime: f64) -> Option<(String, u32, DateTime<Utc>, u64)> {
    if pid == 0 {
        return None;
    }
    let stat_str = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let lparen = stat_str.find('(')?;
    let rparen = stat_str.rfind(')')?;
    let comm = stat_str[lparen + 1..rparen].to_string();
    let rest = stat_str[rparen + 1..].split_whitespace().collect::<Vec<_>>();
    // rest[0] is state, rest[1] is ppid, rest[19] is starttime (in clock ticks)
    let ppid = rest.get(1)?.parse::<u32>().ok()?;
    let starttime_ticks = rest.get(19)?.parse::<u64>().ok()?;

    let clk_tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let clk_tck = if clk_tck > 0 { clk_tck as f64 } else { 100.0 };

    let start_secs_since_boot = (starttime_ticks as f64) / clk_tck;
    let uptime_secs = (sys_uptime - start_secs_since_boot).max(0.0) as u64;
    let started_at = Utc::now() - chrono::Duration::seconds(uptime_secs as i64);

    Some((comm, ppid, started_at, uptime_secs))
}

struct ProcEntry {
    _comm: String,
    ppid: u32,
}

/// Scans /proc to find the srcds process.
/// If main_pid > 0, prioritizes descendants of main_pid.
fn find_srcds_process(main_pid: u32) -> Option<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return None;
    };
    let mut procs = HashMap::new();
    let mut srcds_candidates = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Ok(pid) = name_str.parse::<u32>() {
            if let Ok(stat_str) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                if let (Some(l), Some(r)) = (stat_str.find('('), stat_str.rfind(')')) {
                    let comm = stat_str[l + 1..r].to_string();
                    let ppid = stat_str[r + 1..]
                        .split_whitespace()
                        .nth(1)
                        .and_then(|p| p.parse::<u32>().ok())
                        .unwrap_or(0);

                    if comm.contains("srcds_linux") || comm == "srcds" || comm.contains("srcds_run") {
                        srcds_candidates.push((pid, comm.clone()));
                    }
                    procs.insert(pid, ProcEntry { _comm: comm, ppid });
                }
            }
        }
    }

    if srcds_candidates.is_empty() {
        return None;
    }

    // Function to check if a pid is a descendant of target
    let is_descendant = |mut curr: u32, target: u32| -> bool {
        if target == 0 {
            return false;
        }
        let mut depth = 0;
        while let Some(proc) = procs.get(&curr) {
            if proc.ppid == target {
                return true;
            }
            curr = proc.ppid;
            depth += 1;
            if depth > 10 {
                break;
            }
        }
        false
    };

    // 1. Try to find a descendant of main_pid with "srcds_linux"
    if main_pid > 0 {
        if let Some((pid, _)) = srcds_candidates
            .iter()
            .find(|(pid, comm)| comm.contains("srcds_linux") && is_descendant(*pid, main_pid))
        {
            return Some(*pid);
        }

        // 2. Try any descendant of main_pid in candidates
        if let Some((pid, _)) = srcds_candidates
            .iter()
            .find(|(pid, _)| is_descendant(*pid, main_pid))
        {
            return Some(*pid);
        }
    }

    // 3. Fallback: prefer "srcds_linux" among all candidates
    if let Some((pid, _)) = srcds_candidates
        .iter()
        .find(|(_, comm)| comm.contains("srcds_linux"))
    {
        return Some(*pid);
    }

    // 4. Any candidate
    srcds_candidates.first().map(|(pid, _)| *pid)
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

    let sys_uptime = get_system_uptime().unwrap_or(0.0);

    let (main_comm, main_started_at, main_uptime_secs) = if main_pid > 0 {
        if let Some((comm, _, started_at, uptime_secs)) = get_proc_stat(main_pid, sys_uptime) {
            (Some(comm), Some(started_at), Some(uptime_secs))
        } else {
            let comm = std::fs::read_to_string(format!("/proc/{main_pid}/comm"))
                .ok()
                .map(|s| s.trim().to_string());
            (comm, None, None)
        }
    } else {
        (None, None, None)
    };

    let srcds_pid = find_srcds_process(main_pid);
    let (srcds_comm, srcds_started_at, srcds_uptime_secs) = if let Some(spid) = srcds_pid {
        if let Some((comm, _, started_at, uptime_secs)) = get_proc_stat(spid, sys_uptime) {
            (Some(comm), Some(started_at), Some(uptime_secs))
        } else {
            let comm = std::fs::read_to_string(format!("/proc/{spid}/comm"))
                .ok()
                .map(|s| s.trim().to_string());
            (comm, None, None)
        }
    } else {
        (None, None, None)
    };

    GameServerStatus {
        service: service.to_string(),
        active_state,
        active_state_raw,
        sub_state,
        description,
        main_pid,
        main_comm,
        main_started_at,
        main_uptime_secs,
        srcds_pid,
        srcds_comm,
        srcds_started_at,
        srcds_uptime_secs,
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
