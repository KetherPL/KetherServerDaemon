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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum L4d2CenterSubcommand {
    List,
    Install { name: String },
    Update(L4d2CenterUpdateArgs),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L4d2CenterUpdateArgs {
    pub map_id: Option<u64>,
    pub name: Option<String>,
    pub force: bool,
    pub check_only: bool,
}

const L4D2CENTER_USAGE: &str =
    "Usage: l4d2center list|l|ls | install|i <name> | update|u [id|name] [--check|-c] [--force|-f]";

pub fn parse_l4d2center_subcommand(args: &[&str]) -> Result<L4d2CenterSubcommand, String> {
    let Some(subcommand) = args.first().copied() else {
        return Err(L4D2CENTER_USAGE.to_string());
    };

    match subcommand {
        "list" | "l" | "ls" => Ok(L4d2CenterSubcommand::List),
        "install" | "i" => {
            if args.len() < 2 {
                return Err("Usage: l4d2center install|i <name>".to_string());
            }
            Ok(L4d2CenterSubcommand::Install {
                name: args[1..].join(" "),
            })
        }
        "update" | "u" => Ok(L4d2CenterSubcommand::Update(parse_l4d2center_update_args(
            &args[1..],
        )?)),
        other => Err(format!(
            "Unknown l4d2center subcommand '{other}'. {L4D2CENTER_USAGE}"
        )),
    }
}

pub fn parse_l4d2center_update_args(args: &[&str]) -> Result<L4d2CenterUpdateArgs, String> {
    let mut force = false;
    let mut check_only = false;
    let mut map_id = None;
    let mut name = None;

    for arg in args {
        if *arg == "--force" || *arg == "-f" {
            force = true;
        } else if *arg == "--check" || *arg == "-c" {
            check_only = true;
        } else if let Ok(id) = arg.parse::<u64>() {
            if map_id.is_some() || name.is_some() {
                return Err(
                    "Usage: l4d2center update|u [id|name] [--check|-c] [--force|-f]".to_string(),
                );
            }
            map_id = Some(id);
        } else {
            if map_id.is_some() || name.is_some() {
                return Err(
                    "Usage: l4d2center update|u [id|name] [--check|-c] [--force|-f]".to_string(),
                );
            }
            name = Some(arg.to_string());
        }
    }

    Ok(L4d2CenterUpdateArgs {
        map_id,
        name,
        force,
        check_only,
    })
}
