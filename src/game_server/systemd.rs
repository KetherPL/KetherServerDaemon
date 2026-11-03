// SPDX-License-Identifier: GPL-3.0-only

use std::process::Command;

use tracing::{info, warn};

use super::types::ServiceAction;

/// Build the environment required for `systemctl --user` to work when the
/// process is not running inside a login session (e.g. inside another systemd
/// user service that doesn't inherit the D-Bus socket).
pub(crate) fn systemctl_user_env(uid: u32) -> Vec<(String, String)> {
    vec![
        ("XDG_RUNTIME_DIR".to_string(), format!("/run/user/{uid}")),
        (
            "DBUS_SESSION_BUS_ADDRESS".to_string(),
            format!("unix:path=/run/user/{uid}/bus"),
        ),
    ]
}

fn systemctl_user_command(uid: u32, arguments: &[&str]) -> Command {
    let mut command = Command::new("systemctl");
    command.arg("--user").args(arguments);
    command.envs(systemctl_user_env(uid));
    command
}

pub(super) fn systemctl_user(
    uid: u32,
    arguments: &[&str],
) -> std::io::Result<std::process::Output> {
    systemctl_user_command(uid, arguments).output()
}

/// Build a transient timer whose restart process survives the target service stopping.
/// The caller supplies the user-manager environment before executing the command.
pub(crate) fn delayed_restart_command(service: &str, delay: &str) -> Command {
    const TIMER_ACCURACY: &str = "--timer-property=AccuracySec=100ms";
    let mut command = Command::new("systemd-run");
    command.args([
        "--user",
        "--collect",
        delay,
        TIMER_ACCURACY,
        "--setenv=XDG_RUNTIME_DIR",
        "--setenv=DBUS_SESSION_BUS_ADDRESS",
        "--",
        "systemctl",
        "--user",
        "restart",
        service,
    ]);
    command
}

/// Read a single property from `systemctl --user show`.
pub(super) fn show_property(uid: u32, service: &str, property: &str) -> String {
    let output = systemctl_user(uid, &["show", service, &format!("--property={property}")]);
    let stdout = match output {
        Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
        Err(_) => String::new(),
    };
    parse_property(&stdout, property)
}

fn parse_property(output: &str, property: &str) -> String {
    let prefix = format!("{property}=");
    output
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn property_parsing_preserves_exact_key_matching_and_first_value() {
        assert_eq!(
            parse_property(
                "Other=value\nSubState= running \nSubState=dead\n",
                "SubState"
            ),
            "running"
        );
        assert_eq!(parse_property(" SubState=running\n", "SubState"), "");
        assert_eq!(parse_property("SubState=\n", "SubState"), "");
        assert_eq!(parse_property("", "MainPID"), "");
    }

    #[test]
    fn systemctl_keeps_unit_as_one_argument_and_uses_requested_user_manager() {
        let command = systemctl_user_command(1234, &["restart", "example unit.service"]);
        assert_eq!(command.get_program(), "systemctl");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["--user", "restart", "example unit.service"].map(std::ffi::OsStr::new)
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
}
