// SPDX-License-Identifier: GPL-3.0-only

use super::process::{find_srcds_process, get_system_uptime, process_details};
use super::systemd::{show_property, systemctl_user};
use super::types::{GameServerStatus, UnitActiveState};

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

    let main_process = process_details(main_pid, sys_uptime);
    let srcds_pid = find_srcds_process(main_pid);
    let srcds_process = srcds_pid
        .map(|pid| process_details(pid, sys_uptime))
        .unwrap_or_default();

    GameServerStatus {
        service: service.to_string(),
        active_state,
        active_state_raw,
        sub_state,
        description,
        main_pid,
        main_comm: main_process.command_name,
        main_started_at: main_process.started_at,
        main_uptime_secs: main_process.uptime_secs,
        srcds_pid,
        srcds_comm: srcds_process.command_name,
        srcds_started_at: srcds_process.started_at,
        srcds_uptime_secs: srcds_process.uptime_secs,
    }
}
