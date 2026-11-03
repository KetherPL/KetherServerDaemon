// SPDX-License-Identifier: GPL-3.0-only
//! Control the L4D2 game server via systemctl --user.

mod process;
mod status;
mod systemd;
mod types;

pub use status::game_server_status;
pub use systemd::game_server_action;
pub(crate) use systemd::{delayed_restart_command, systemctl_user_env};
pub use types::{DEFAULT_L4D2_SERVICE, GameServerStatus, ServiceAction, UnitActiveState};

#[cfg(test)]
mod tests {
    use super::process::{get_proc_stat, get_system_uptime};
    use super::*;
    use chrono::Utc;

    #[test]
    fn unit_states_preserve_display_and_wire_values() {
        for (state, text) in [
            (UnitActiveState::Active, "active"),
            (UnitActiveState::Inactive, "inactive"),
            (UnitActiveState::Activating, "activating"),
            (UnitActiveState::Deactivating, "deactivating"),
            (UnitActiveState::Failed, "failed"),
            (UnitActiveState::Unknown, "unknown"),
        ] {
            assert_eq!(state.to_string(), text);
            assert_eq!(UnitActiveState::from(format!(" {text}\n").as_str()), state);
            assert_eq!(serde_json::to_value(&state).expect("serialize state"), text);
        }
        assert_eq!(
            UnitActiveState::from("unexpected"),
            UnitActiveState::Unknown
        );
    }

    #[test]
    fn service_actions_preserve_systemctl_verbs() {
        assert_eq!(ServiceAction::Start.to_string(), "start");
        assert_eq!(ServiceAction::Stop.to_string(), "stop");
        assert_eq!(ServiceAction::Restart.to_string(), "restart");
    }

    #[test]
    fn user_manager_environment_uses_requested_uid() {
        assert_eq!(
            systemctl_user_env(1234),
            vec![
                ("XDG_RUNTIME_DIR".to_string(), "/run/user/1234".to_string()),
                (
                    "DBUS_SESSION_BUS_ADDRESS".to_string(),
                    "unix:path=/run/user/1234/bus".to_string()
                ),
            ]
        );
    }

    #[test]
    fn process_stat_handles_zero_and_reads_current_process() {
        assert!(get_proc_stat(0, 0.0).is_none());
        let system_uptime = get_system_uptime().expect("system uptime");
        let (command_name, _, started_at, uptime_secs) =
            get_proc_stat(std::process::id(), system_uptime).expect("current process stat");
        assert!(!command_name.is_empty());
        assert!(started_at <= Utc::now());
        assert!(uptime_secs <= system_uptime as u64);
    }
}
