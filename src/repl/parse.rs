// SPDX-License-Identifier: GPL-3.0-only
use crate::map_installer::DiscoveryMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallTarget {
    Workshop(u64),
    Url(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateArgs {
    pub map_id: Option<u64>,
    pub force: bool,
    pub check_only: bool,
}

pub fn parse_map_id(raw: &str) -> Result<u64, String> {
    raw.parse::<u64>()
        .map_err(|err| format!("Invalid map ID '{raw}': {err}"))
}

pub fn parse_install_source(token: &str) -> Result<InstallTarget, String> {
    if token.chars().all(|ch| ch.is_ascii_digit()) {
        token
            .parse::<u64>()
            .map(InstallTarget::Workshop)
            .map_err(|err| format!("Invalid workshop ID '{token}': {err}"))
    } else {
        Ok(InstallTarget::Url(token.to_string()))
    }
}

pub fn parse_discovery_mode(arg: Option<&str>) -> Result<DiscoveryMode, String> {
    match arg {
        None => Ok(DiscoveryMode::Add),
        Some("u") | Some("update") => Ok(DiscoveryMode::Update),
        Some("U") | Some("forceupdate") => Ok(DiscoveryMode::ForceUpdate),
        Some(other) => Err(format!(
            "Unknown discovery argument '{other}'. Usage: d [u|U] (u=update, U=force update)."
        )),
    }
}

pub fn parse_update_args(args: &[&str]) -> Result<UpdateArgs, String> {
    let mut force = false;
    let mut check_only = false;
    let mut map_id = None;

    for arg in args {
        if *arg == "--force" {
            force = true;
        } else if *arg == "--check" {
            check_only = true;
        } else if let Ok(id) = arg.parse::<u64>() {
            if map_id.is_some() {
                return Err("Usage: update [id] [--check] [--force]".to_string());
            }
            map_id = Some(id);
        } else {
            return Err("Usage: update [id] [--check] [--force]".to_string());
        }
    }

    Ok(UpdateArgs {
        map_id,
        force,
        check_only,
    })
}
