// SPDX-License-Identifier: GPL-3.0-only

use std::collections::HashMap;

use chrono::{DateTime, Utc};

const FALLBACK_CLOCK_TICKS_PER_SECOND: f64 = 100.0;
const PARENT_PID_FIELD_INDEX: usize = 1;
const START_TIME_FIELD_INDEX: usize = 19;
const MAX_ANCESTOR_DEPTH: usize = 10;
const SRCDS_LINUX_COMMAND: &str = "srcds_linux";

#[derive(Default)]
pub(super) struct ProcessDetails {
    pub command_name: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub uptime_secs: Option<u64>,
}

/// Reads system uptime from /proc/uptime in seconds.
pub(super) fn get_system_uptime() -> Option<f64> {
    let content = std::fs::read_to_string("/proc/uptime").ok()?;
    content.split_whitespace().next()?.parse::<f64>().ok()
}

/// Reads process stat information: (command name, parent PID, started_at, uptime_secs).
pub(super) fn get_proc_stat(
    pid: u32,
    system_uptime: f64,
) -> Option<(String, u32, DateTime<Utc>, u64)> {
    if pid == 0 {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let clock_ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    parse_proc_stat(&stat, system_uptime, clock_ticks)
}

fn parse_proc_stat(
    stat: &str,
    system_uptime: f64,
    clock_ticks: libc::c_long,
) -> Option<(String, u32, DateTime<Utc>, u64)> {
    let opening_parenthesis = stat.find('(')?;
    let closing_parenthesis = stat.rfind(')')?;
    let command_name = stat[opening_parenthesis + 1..closing_parenthesis].to_string();
    let fields = stat[closing_parenthesis + 1..]
        .split_whitespace()
        .collect::<Vec<_>>();
    let parent_pid = fields.get(PARENT_PID_FIELD_INDEX)?.parse::<u32>().ok()?;
    let start_time_ticks = fields.get(START_TIME_FIELD_INDEX)?.parse::<u64>().ok()?;

    let clock_ticks_per_second = if clock_ticks > 0 {
        clock_ticks as f64
    } else {
        FALLBACK_CLOCK_TICKS_PER_SECOND
    };
    let start_seconds_since_boot = start_time_ticks as f64 / clock_ticks_per_second;
    let uptime_secs = (system_uptime - start_seconds_since_boot).max(0.0) as u64;
    let started_at = Utc::now() - chrono::Duration::seconds(uptime_secs as i64);

    Some((command_name, parent_pid, started_at, uptime_secs))
}

pub(super) fn process_details(pid: u32, system_uptime: f64) -> ProcessDetails {
    if pid == 0 {
        return ProcessDetails::default();
    }
    if let Some((command_name, _, started_at, uptime_secs)) = get_proc_stat(pid, system_uptime) {
        return ProcessDetails {
            command_name: Some(command_name),
            started_at: Some(started_at),
            uptime_secs: Some(uptime_secs),
        };
    }
    let command_name = std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|name| name.trim().to_string());
    ProcessDetails {
        command_name,
        ..ProcessDetails::default()
    }
}

fn parse_process_parent(stat: &str) -> Option<(String, u32)> {
    let opening_parenthesis = stat.find('(')?;
    let closing_parenthesis = stat.rfind(')')?;
    let command_name = stat[opening_parenthesis + 1..closing_parenthesis].to_string();
    let parent_pid = stat[closing_parenthesis + 1..]
        .split_whitespace()
        .nth(PARENT_PID_FIELD_INDEX)
        .and_then(|field| field.parse::<u32>().ok())
        .unwrap_or(0);
    Some((command_name, parent_pid))
}

fn is_descendant(parent_process_ids: &HashMap<u32, u32>, mut pid: u32, target_pid: u32) -> bool {
    if target_pid == 0 {
        return false;
    }
    let mut depth = 0;
    while let Some(&parent_pid) = parent_process_ids.get(&pid) {
        if parent_pid == target_pid {
            return true;
        }
        pid = parent_pid;
        depth += 1;
        if depth > MAX_ANCESTOR_DEPTH {
            break;
        }
    }
    false
}

fn select_srcds_process(
    main_pid: u32,
    parent_process_ids: &HashMap<u32, u32>,
    candidates: &[(u32, String)],
) -> Option<u32> {
    if main_pid > 0 {
        if let Some((pid, _)) = candidates.iter().find(|(pid, command_name)| {
            command_name.contains(SRCDS_LINUX_COMMAND)
                && is_descendant(parent_process_ids, *pid, main_pid)
        }) {
            return Some(*pid);
        }
        if let Some((pid, _)) = candidates
            .iter()
            .find(|(pid, _)| is_descendant(parent_process_ids, *pid, main_pid))
        {
            return Some(*pid);
        }
    }
    candidates
        .iter()
        .find(|(_, command_name)| command_name.contains(SRCDS_LINUX_COMMAND))
        .or_else(|| candidates.first())
        .map(|(pid, _)| *pid)
}

/// Scans /proc, preferring SRCDS descendants of the service's main process.
pub(super) fn find_srcds_process(main_pid: u32) -> Option<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return None;
    };
    let mut parent_process_ids = HashMap::new();
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let Some((command_name, parent_pid)) = parse_process_parent(&stat) else {
            continue;
        };
        if command_name.contains(SRCDS_LINUX_COMMAND)
            || command_name == "srcds"
            || command_name.contains("srcds_run")
        {
            candidates.push((pid, command_name));
        }
        parent_process_ids.insert(pid, parent_pid);
    }
    select_srcds_process(main_pid, &parent_process_ids, &candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(command_name: &str, parent_pid: &str, start_ticks: &str) -> String {
        let mut fields = vec!["0"; START_TIME_FIELD_INDEX + 1];
        fields[0] = "S";
        fields[PARENT_PID_FIELD_INDEX] = parent_pid;
        fields[START_TIME_FIELD_INDEX] = start_ticks;
        format!("123 ({command_name}) {}", fields.join(" "))
    }

    #[test]
    fn stat_parsing_handles_command_parentheses_and_clock_tick_fallback() {
        let contents = stat("srcds (server name)", "7", "2500");
        for clock_ticks in [100, 0, -1] {
            let (command_name, parent_pid, _, uptime_secs) =
                parse_proc_stat(&contents, 30.0, clock_ticks).expect("stat");
            assert_eq!(command_name, "srcds (server name)");
            assert_eq!(parent_pid, 7);
            assert_eq!(uptime_secs, 5);
        }
        assert_eq!(parse_proc_stat(&contents, 20.0, 100).expect("stat").3, 0);
    }

    #[test]
    fn incomplete_stat_preserves_different_parent_scan_and_detail_fallbacks() {
        assert!(parse_proc_stat("", 0.0, 100).is_none());
        assert!(parse_proc_stat("123 (srcds) S 7", 30.0, 100).is_none());
        assert_eq!(
            parse_process_parent("123 (srcds) S invalid"),
            Some(("srcds".to_string(), 0))
        );
        assert!(parse_proc_stat(&stat("srcds", "invalid", "2500"), 30.0, 100).is_none());
        assert!(parse_proc_stat(&stat("srcds", "7", "invalid"), 30.0, 100).is_none());
    }

    #[test]
    fn selection_preserves_descendant_priority_and_candidate_order() {
        let parents = HashMap::from([(10, 100), (11, 200), (12, 101), (101, 100), (13, 100)]);
        let candidates = vec![
            (10, "srcds_run".to_string()),
            (11, "srcds_linux".to_string()),
            (12, "srcds_linux64".to_string()),
            (13, "srcds_linux".to_string()),
        ];
        assert_eq!(select_srcds_process(100, &parents, &candidates), Some(12));
        assert_eq!(
            select_srcds_process(100, &parents, &candidates[..2]),
            Some(10)
        );
        assert_eq!(select_srcds_process(999, &parents, &candidates), Some(11));
        assert_eq!(select_srcds_process(0, &parents, &candidates), Some(11));
        assert_eq!(
            select_srcds_process(0, &parents, &candidates[..1]),
            Some(10)
        );
        assert_eq!(select_srcds_process(100, &parents, &[]), None);
    }

    #[test]
    fn ancestry_preserves_existing_limit_and_terminates_on_cycles() {
        let parents = (1..=12).map(|pid| (pid, pid + 1)).collect();
        assert!(is_descendant(&parents, 1, 12));
        assert!(!is_descendant(&parents, 1, 13));
        assert!(!is_descendant(&parents, 1, 0));
        assert!(!is_descendant(&HashMap::from([(1, 2), (2, 1)]), 1, 100));
    }

    #[test]
    fn zero_pid_has_no_process_details() {
        let details = process_details(0, 0.0);
        assert!(details.command_name.is_none());
        assert!(details.started_at.is_none());
        assert!(details.uptime_secs.is_none());
    }
}
