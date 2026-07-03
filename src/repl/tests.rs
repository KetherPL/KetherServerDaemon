// SPDX-License-Identifier: GPL-3.0-only
use crate::map_installer::DiscoveryMode;
use crate::registry::SourceKind;

use super::format::source_kind_label;
use super::parse::{
    parse_discovery_mode, parse_install_source, parse_l4d2center_subcommand,
    parse_l4d2center_update_args, parse_map_id, parse_update_args, InstallTarget,
    L4d2CenterSubcommand, UpdateArgs,
};

#[test]
fn parse_map_id_accepts_valid_id() {
    assert_eq!(parse_map_id("42").unwrap(), 42);
}

#[test]
fn parse_map_id_rejects_invalid_id() {
    let err = parse_map_id("not-a-number").unwrap_err();
    assert!(err.contains("Invalid map ID"));
    assert!(err.contains("not-a-number"));
}

#[test]
fn parse_install_source_workshop_id() {
    assert_eq!(
        parse_install_source("123456789").unwrap(),
        InstallTarget::Workshop(123_456_789)
    );
}

#[test]
fn parse_install_source_url() {
    assert_eq!(
        parse_install_source("https://example.com/map.zip").unwrap(),
        InstallTarget::Url("https://example.com/map.zip".to_string())
    );
}

#[test]
fn parse_install_source_rejects_invalid_workshop_id() {
    let oversized = "1".repeat(30);
    let err = parse_install_source(&oversized).unwrap_err();
    assert!(err.contains("Invalid workshop ID"));
}

#[test]
fn parse_discovery_mode_defaults_to_add() {
    assert_eq!(parse_discovery_mode(None).unwrap(), DiscoveryMode::Add);
}

#[test]
fn parse_discovery_mode_update_variants() {
    assert_eq!(parse_discovery_mode(Some("u")).unwrap(), DiscoveryMode::Update);
    assert_eq!(
        parse_discovery_mode(Some("update")).unwrap(),
        DiscoveryMode::Update
    );
    assert_eq!(
        parse_discovery_mode(Some("U")).unwrap(),
        DiscoveryMode::ForceUpdate
    );
    assert_eq!(
        parse_discovery_mode(Some("forceupdate")).unwrap(),
        DiscoveryMode::ForceUpdate
    );
}

#[test]
fn parse_discovery_mode_rejects_unknown_arg() {
    let err = parse_discovery_mode(Some("invalid")).unwrap_err();
    assert!(err.contains("Unknown discovery argument"));
}

#[test]
fn parse_update_args_flags_only() {
    assert_eq!(
        parse_update_args(&["--check", "--force"]).unwrap(),
        UpdateArgs {
            map_id: None,
            force: true,
            check_only: true,
        }
    );
}

#[test]
fn parse_update_args_id_and_flags() {
    assert_eq!(
        parse_update_args(&["5", "--check"]).unwrap(),
        UpdateArgs {
            map_id: Some(5),
            force: false,
            check_only: true,
        }
    );
}

#[test]
fn parse_update_args_rejects_duplicate_id() {
    let err = parse_update_args(&["1", "2"]).unwrap_err();
    assert!(err.contains("Usage: update"));
}

#[test]
fn parse_update_args_rejects_unknown_token() {
    let err = parse_update_args(&["--nope"]).unwrap_err();
    assert!(err.contains("Usage: update"));
}

#[test]
fn source_kind_label_all_variants() {
    assert_eq!(source_kind_label(SourceKind::Workshop), "workshop");
    assert_eq!(source_kind_label(SourceKind::SirPlease), "sirplease");
    assert_eq!(source_kind_label(SourceKind::L4d2Center), "l4d2center");
    assert_eq!(source_kind_label(SourceKind::Other), "other");
}

#[test]
fn parse_l4d2center_install_subcommand() {
    assert_eq!(
        parse_l4d2center_subcommand(&["install", "widebox1.vpk"]).unwrap(),
        L4d2CenterSubcommand::Install {
            name: "widebox1.vpk".to_string()
        }
    );
}

#[test]
fn parse_l4d2center_list_subcommand() {
    assert_eq!(
        parse_l4d2center_subcommand(&["list"]).unwrap(),
        L4d2CenterSubcommand::List
    );
    assert_eq!(
        parse_l4d2center_subcommand(&["l"]).unwrap(),
        L4d2CenterSubcommand::List
    );
    assert_eq!(
        parse_l4d2center_subcommand(&["ls"]).unwrap(),
        L4d2CenterSubcommand::List
    );
}

#[test]
fn parse_l4d2center_update_subcommand_shortcuts() {
    use crate::repl::parse::L4d2CenterUpdateArgs;

    assert_eq!(
        parse_l4d2center_subcommand(&["u", "--check"]).unwrap(),
        L4d2CenterSubcommand::Update(L4d2CenterUpdateArgs {
            map_id: None,
            name: None,
            force: false,
            check_only: true,
        })
    );
    assert_eq!(
        parse_l4d2center_update_args(&["-c"]).unwrap(),
        L4d2CenterUpdateArgs {
            map_id: None,
            name: None,
            force: false,
            check_only: true,
        }
    );
    assert_eq!(
        parse_l4d2center_subcommand(&["i", "widebox1.vpk"]).unwrap(),
        L4d2CenterSubcommand::Install {
            name: "widebox1.vpk".to_string()
        }
    );
}
