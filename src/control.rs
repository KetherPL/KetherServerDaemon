// SPDX-License-Identifier: GPL-3.0-only
//! Process lifecycle helpers: start time (for status/uptime) and daemon restarts.
//!
//! A soft restart re-executes the daemon binary *in place* (`exec`, same PID, same terminal or
//! `screen` session, same arguments), so it works the same whether the daemon runs in a
//! terminal, under `screen`, or as a systemd service. If the binary was replaced on disk
//! (e.g. after `cargo build --release`), the new build is what starts.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::time::Instant;

use chrono::{DateTime, Utc};

static STARTED: OnceLock<(Instant, DateTime<Utc>)> = OnceLock::new();

/// The systemd user service that runs the daemon.
pub const DAEMON_SERVICE: &str = "ksd.service";
const SERVICE_RESTART_DELAY: &str = "--on-active=1s";

fn service_restart_command(uid: u32) -> Command {
    let mut command =
        crate::game_server::delayed_restart_command(DAEMON_SERVICE, SERVICE_RESTART_DELAY);
    command.envs(crate::game_server::systemctl_user_env(uid));
    command
}

fn check_service_restart_output(output: std::io::Result<Output>) -> Result<(), String> {
    let output =
        output.map_err(|error| format!("Failed to schedule {DAEMON_SERVICE} restart: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Failed to schedule {DAEMON_SERVICE} restart ({}): {} {}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        ));
    }
    Ok(())
}

/// Schedule a restart through the same user manager as the game server.
/// Returns only after the manager accepts the transient timer.
pub fn restart_service(uid: u32) -> Result<(), String> {
    check_service_restart_output(service_restart_command(uid).output())?;
    tracing::info!(
        uid,
        service = DAEMON_SERVICE,
        "Daemon service restart scheduled"
    );
    Ok(())
}

/// Records the process start time. Call once at the beginning of `main`.
pub fn mark_started() {
    let _ = STARTED.set((Instant::now(), Utc::now()));
}

/// When the process (re)started.
pub fn started_at() -> DateTime<Utc> {
    STARTED.get().map(|(_, at)| *at).unwrap_or_else(Utc::now)
}

/// Seconds since the process (re)started.
pub fn uptime_secs() -> u64 {
    STARTED
        .get()
        .map(|(instant, _)| instant.elapsed().as_secs())
        .unwrap_or(0)
}

/// Picks the executable to re-run. `/proc/self/exe` reports `"<path> (deleted)"` once the
/// binary has been replaced on disk; in that case the path without the suffix (the new
/// build) is used. Falls back to `argv[0]`.
fn soft_restart_executable(
    current_exe: Option<PathBuf>,
    argv0: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(exe) = current_exe {
        if exe.exists() {
            return Some(exe);
        }
        if let Some(stripped) = exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
            let stripped = PathBuf::from(stripped);
            if stripped.exists() {
                return Some(stripped);
            }
        }
    }
    argv0.map(PathBuf::from)
}

/// Replaces the current process with a fresh instance of the daemon (same arguments).
///
/// Only returns if starting the new instance failed.
pub fn soft_restart_process() -> std::io::Error {
    // A REPL session may have put the terminal into raw mode; leave it in a sane state for
    // the new process. Ignored when not attached to a terminal.
    let _ = crossterm::terminal::disable_raw_mode();

    let mut args = std::env::args_os();
    let argv0 = args.next();
    let Some(exe) = soft_restart_executable(std::env::current_exe().ok(), argv0) else {
        return std::io::Error::other("cannot determine the daemon executable path");
    };
    let mut command = Command::new(exe);
    command.args(args);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.exec()
    }
    #[cfg(not(unix))]
    {
        match command.spawn() {
            Ok(_) => std::process::exit(0),
            Err(err) => err,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_restart_executable_prefers_existing_current_exe() {
        let dir = tempfile::tempdir().expect("tempdir");
        let exe = dir.path().join("KetherServerDaemon");
        std::fs::write(&exe, b"").expect("write");
        assert_eq!(
            soft_restart_executable(Some(exe.clone()), Some("argv0".into())),
            Some(exe)
        );
    }

    #[test]
    fn soft_restart_executable_strips_deleted_suffix_after_rebuild() {
        let dir = tempfile::tempdir().expect("tempdir");
        let exe = dir.path().join("KetherServerDaemon");
        std::fs::write(&exe, b"").expect("write");
        let deleted = PathBuf::from(format!("{} (deleted)", exe.display()));
        assert_eq!(soft_restart_executable(Some(deleted), None), Some(exe));
    }

    #[test]
    fn soft_restart_executable_falls_back_to_argv0() {
        assert_eq!(
            soft_restart_executable(
                Some(PathBuf::from("/nonexistent/ksd")),
                Some("./ksd".into())
            ),
            Some(PathBuf::from("./ksd"))
        );
    }

    #[test]
    fn service_restart_uses_user_manager_and_fixed_daemon_unit() {
        let command = service_restart_command(1234);
        assert_eq!(command.get_program(), "systemd-run");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "--user",
                "--collect",
                "--on-active=1s",
                "--timer-property=AccuracySec=100ms",
                "--setenv=XDG_RUNTIME_DIR",
                "--setenv=DBUS_SESSION_BUS_ADDRESS",
                "--",
                "systemctl",
                "--user",
                "restart",
                "ksd.service"
            ]
            .map(std::ffi::OsStr::new)
        );
        let environment = command
            .get_envs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            environment[std::ffi::OsStr::new("XDG_RUNTIME_DIR")],
            Some(std::ffi::OsStr::new("/run/user/1234"))
        );
        assert_eq!(
            environment[std::ffi::OsStr::new("DBUS_SESSION_BUS_ADDRESS")],
            Some(std::ffi::OsStr::new("unix:path=/run/user/1234/bus"))
        );
    }

    #[test]
    fn soft_restart_executable_returns_none_when_no_path_is_available() {
        assert_eq!(soft_restart_executable(None, None), None);
    }

    #[test]
    fn soft_restart_executable_uses_argv0_when_rebuilt_binary_is_missing() {
        let directory = tempfile::tempdir().expect("tempdir");
        let deleted = directory.path().join("missing (deleted)");
        assert_eq!(
            soft_restart_executable(Some(deleted), Some("./fallback".into())),
            Some(PathBuf::from("./fallback"))
        );
    }

    #[test]
    fn restart_scheduler_spawn_error_preserves_message() {
        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "scheduler unavailable");
        assert_eq!(
            check_service_restart_output(Err(error)),
            Err("Failed to schedule ksd.service restart: scheduler unavailable".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn restart_scheduler_preserves_exit_status_and_output_order() {
        use std::os::unix::process::ExitStatusExt;
        let failed = Output {
            status: std::process::ExitStatus::from_raw(1 << 8),
            stderr: b"manager rejected restart".to_vec(),
            stdout: b"additional output".to_vec(),
        };
        let expected = format!(
            "Failed to schedule ksd.service restart ({}): manager rejected restart additional output",
            failed.status
        );
        assert_eq!(check_service_restart_output(Ok(failed)), Err(expected));
        let accepted = Output {
            status: std::process::ExitStatus::from_raw(0),
            stderr: b"timer created".to_vec(),
            stdout: Vec::new(),
        };
        assert_eq!(check_service_restart_output(Ok(accepted)), Ok(()));
    }

    #[test]
    fn uptime_is_tracked_after_mark_started() {
        mark_started();
        assert!(started_at() <= Utc::now());
        assert!(uptime_secs() < 60);
    }
}
