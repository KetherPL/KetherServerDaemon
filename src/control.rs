// SPDX-License-Identifier: GPL-3.0-only
//! Process lifecycle helpers: start time (for status/uptime) and self-restart.
//!
//! A restart re-executes the daemon binary *in place* (`exec`, same PID, same terminal or
//! `screen` session, same arguments), so it works the same whether the daemon runs in a
//! terminal, under `screen`, or as a systemd service. If the binary was replaced on disk
//! (e.g. after `cargo build --release`), the new build is what starts.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use chrono::{DateTime, Utc};

static STARTED: OnceLock<(Instant, DateTime<Utc>)> = OnceLock::new();

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
fn restart_executable(current_exe: Option<PathBuf>, argv0: Option<OsString>) -> Option<PathBuf> {
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
pub fn restart_process() -> std::io::Error {
    // A REPL session may have put the terminal into raw mode; leave it in a sane state for
    // the new process. Ignored when not attached to a terminal.
    let _ = crossterm::terminal::disable_raw_mode();

    let mut args = std::env::args_os();
    let argv0 = args.next();
    let Some(exe) = restart_executable(std::env::current_exe().ok(), argv0) else {
        return std::io::Error::other("cannot determine the daemon executable path");
    };
    let mut command = std::process::Command::new(exe);
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
    fn restart_executable_prefers_existing_current_exe() {
        let dir = tempfile::tempdir().expect("tempdir");
        let exe = dir.path().join("KetherServerDaemon");
        std::fs::write(&exe, b"").expect("write");
        assert_eq!(
            restart_executable(Some(exe.clone()), Some("argv0".into())),
            Some(exe)
        );
    }

    #[test]
    fn restart_executable_strips_deleted_suffix_after_rebuild() {
        let dir = tempfile::tempdir().expect("tempdir");
        let exe = dir.path().join("KetherServerDaemon");
        std::fs::write(&exe, b"").expect("write");
        let deleted = PathBuf::from(format!("{} (deleted)", exe.display()));
        assert_eq!(restart_executable(Some(deleted), None), Some(exe));
    }

    #[test]
    fn restart_executable_falls_back_to_argv0() {
        assert_eq!(
            restart_executable(
                Some(PathBuf::from("/nonexistent/ksd")),
                Some("./ksd".into())
            ),
            Some(PathBuf::from("./ksd"))
        );
    }

    #[test]
    fn uptime_is_tracked_after_mark_started() {
        mark_started();
        assert!(started_at() <= Utc::now());
        assert!(uptime_secs() < 60);
    }
}
