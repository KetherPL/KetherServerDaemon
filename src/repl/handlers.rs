// SPDX-License-Identifier: GPL-3.0-only
use super::format::{
    print_compact_report, print_discovery_report, print_l4d2center_catalog,
    print_l4d2center_check_report, print_l4d2center_update_report, print_map_detail,
    print_map_entry, print_workshop_check_report, print_workshop_update_report,
};
use super::parse::{
    parse_discovery_mode, parse_install_source, parse_l4d2center_subcommand, parse_map_id,
    parse_update_args, InstallTarget, L4d2CenterSubcommand,
};
use super::session::Repl;
use super::runtime::{block_on_installer, require_installer};

impl Repl {
    pub(super) fn handle_list_maps(&self, runtime_handle: &tokio::runtime::Handle) {
        let Some(installer) = require_installer(&self.installer) else {
            return;
        };

        match block_on_installer(runtime_handle, installer, installer.registry().list_maps()) {
            Ok(maps) => {
                if maps.is_empty() {
                    println!("No maps installed.");
                    return;
                }

                println!("Installed maps:");
                for map in maps {
                    print_map_entry(&map);
                }
            }
            Err(err) => {
                eprintln!("Failed to list maps: {err}");
            }
        }
    }

    pub(super) fn handle_install(&self, runtime_handle: &tokio::runtime::Handle, args: &[&str]) {
        let Some(installer) = require_installer(&self.installer) else {
            return;
        };

        if args.is_empty() {
            println!("Usage: install <url|workshop_id> [name]");
            return;
        }

        let provided_name = if args.len() > 1 {
            Some(args[1..].join(" "))
        } else {
            None
        };

        let target = match parse_install_source(args[0]) {
            Ok(target) => target,
            Err(err) => {
                eprintln!("{err}");
                return;
            }
        };

        let result = match target {
            InstallTarget::Workshop(workshop_id) => block_on_installer(
                runtime_handle,
                installer,
                installer.install_from_workshop_id(workshop_id, provided_name),
            ),
            InstallTarget::Url(url) => block_on_installer(
                runtime_handle,
                installer,
                installer.install_from_url(url, provided_name),
            ),
        };

        match result {
            Ok(map_entry) => {
                println!(
                    "Installed map #{}: {} ({})",
                    map_entry.id, map_entry.name, map_entry.installed_path
                );
            }
            Err(err) => {
                eprintln!("Install failed: {err}");
            }
        }
    }

    pub(super) fn handle_remove(
        &self,
        runtime_handle: &tokio::runtime::Handle,
        id_arg: Option<&str>,
    ) {
        let Some(installer) = require_installer(&self.installer) else {
            return;
        };

        let Some(id_raw) = id_arg else {
            println!("Usage: remove <id>");
            return;
        };

        let map_id = match parse_map_id(id_raw) {
            Ok(map_id) => map_id,
            Err(err) => {
                eprintln!("{err}");
                return;
            }
        };

        match block_on_installer(runtime_handle, installer, installer.uninstall_map(map_id)) {
            Ok(()) => println!("Removed map #{map_id}."),
            Err(err) => eprintln!("Failed to remove map #{map_id}: {err}"),
        }
    }

    pub(super) fn handle_discover(&self, runtime_handle: &tokio::runtime::Handle, args: &[&str]) {
        let Some(installer) = require_installer(&self.installer) else {
            return;
        };

        let mode = match parse_discovery_mode(args.first().copied()) {
            Ok(mode) => mode,
            Err(err) => {
                eprintln!("{err}");
                return;
            }
        };

        match block_on_installer(runtime_handle, installer, installer.discover_maps(mode)) {
            Ok(report) => print_discovery_report(report),
            Err(err) => {
                eprintln!("Discovery failed: {err}");
            }
        }
    }

    pub(super) fn handle_compact(&self, runtime_handle: &tokio::runtime::Handle) {
        let Some(installer) = require_installer(&self.installer) else {
            return;
        };

        match block_on_installer(runtime_handle, installer, installer.compact_registry()) {
            Ok(report) => print_compact_report(report),
            Err(err) => {
                eprintln!("Compact failed: {err}");
            }
        }
    }

    pub(super) fn handle_update(&self, runtime_handle: &tokio::runtime::Handle, args: &[&str]) {
        let Some(installer) = require_installer(&self.installer) else {
            return;
        };

        let update_args = match parse_update_args(args) {
            Ok(update_args) => update_args,
            Err(err) => {
                println!("{err}");
                return;
            }
        };

        match block_on_installer(
            runtime_handle,
            installer,
            installer.update_workshop_maps(
                update_args.map_id,
                update_args.force,
                update_args.check_only,
            ),
        ) {
            Ok(report) => {
                if update_args.check_only {
                    print_workshop_check_report(&report);
                } else {
                    print_workshop_update_report(&report);
                }
            }
            Err(err) => eprintln!("Workshop update failed: {err}"),
        }
    }

    pub(super) fn handle_info(
        &self,
        runtime_handle: &tokio::runtime::Handle,
        id_arg: Option<&str>,
    ) {
        let Some(installer) = require_installer(&self.installer) else {
            return;
        };

        let Some(id_raw) = id_arg else {
            println!("Usage: info <id>");
            return;
        };

        let map_id = match parse_map_id(id_raw) {
            Ok(map_id) => map_id,
            Err(err) => {
                eprintln!("{err}");
                return;
            }
        };

        match block_on_installer(runtime_handle, installer, installer.registry().get_map(map_id)) {
            Ok(Some(map)) => print_map_detail(&map),
            Ok(None) => println!("Map #{map_id} not found."),
            Err(err) => eprintln!("Failed to load map #{map_id}: {err}"),
        }
    }

    pub(super) fn handle_modify(&self, runtime_handle: &tokio::runtime::Handle, args: &[&str]) {
        let Some(installer) = require_installer(&self.installer) else {
            return;
        };

        if args.len() < 3 {
            println!(
                "Usage: modify <id> <field> <value>\nEditable fields: name, source_url, version, source_kind, workshop_id"
            );
            return;
        }

        let map_id = match parse_map_id(args[0]) {
            Ok(map_id) => map_id,
            Err(err) => {
                eprintln!("{err}");
                return;
            }
        };

        let field = args[1];
        let value = args[2..].join(" ");

        match block_on_installer(
            runtime_handle,
            installer,
            installer.modify_map_field(map_id, field, &value),
        ) {
            Ok(map) => {
                println!("Updated map #{}:", map.id);
                print_map_detail(&map);
            }
            Err(err) => eprintln!("Modify failed: {err}"),
        }
    }

    pub(super) fn handle_l4d2center(
        &self,
        runtime_handle: &tokio::runtime::Handle,
        args: &[&str],
    ) {
        let Some(installer) = require_installer(&self.installer) else {
            return;
        };

        if self.l4d2center_index_url.is_empty() {
            eprintln!("L4D2Center index URL is not configured.");
            return;
        }

        let subcommand = match parse_l4d2center_subcommand(args) {
            Ok(subcommand) => subcommand,
            Err(err) => {
                eprintln!("{err}");
                return;
            }
        };

        let index_url = self.l4d2center_index_url.clone();

        match subcommand {
            L4d2CenterSubcommand::List => {
                match block_on_installer(
                    runtime_handle,
                    installer,
                    installer.list_l4d2center_catalog(&index_url),
                ) {
                    Ok(catalog) => print_l4d2center_catalog(&catalog),
                    Err(err) => eprintln!("Failed to list L4D2Center catalog: {err}"),
                }
            }
            L4d2CenterSubcommand::Install { name } => {
                match block_on_installer(
                    runtime_handle,
                    installer,
                    installer.install_l4d2center_by_name(&index_url, &name),
                ) {
                    Ok(map_entry) => {
                        println!(
                            "Installed L4D2Center map #{}: {} ({})",
                            map_entry.id, map_entry.name, map_entry.installed_path
                        );
                    }
                    Err(err) => eprintln!("L4D2Center install failed: {err}"),
                }
            }
            L4d2CenterSubcommand::Update(update_args) => {
                match block_on_installer(
                    runtime_handle,
                    installer,
                    installer.update_l4d2center_maps(
                        &index_url,
                        update_args.map_id,
                        update_args.name.as_deref(),
                        update_args.force,
                        update_args.check_only,
                    ),
                ) {
                    Ok(report) => {
                        if update_args.check_only {
                            print_l4d2center_check_report(&report);
                        } else {
                            print_l4d2center_update_report(&report);
                        }
                    }
                    Err(err) => eprintln!("L4D2Center update failed: {err}"),
                }
            }
        }
    }
}
