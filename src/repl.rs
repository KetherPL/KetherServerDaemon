// SPDX-License-Identifier: GPL-3.0-only

use std::sync::Arc;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use reedline::{DefaultPrompt, DefaultPromptSegment, Reedline, Signal};
use tokio::sync::mpsc;

use crate::map_installer::{
    CompactReport, DiscoveryMode, DiscoveryReport, MapInstallationService, WorkshopUpdateReport,
};
use crate::registry::{MapEntry, SourceKind};

#[derive(Debug, Clone, Copy)]
pub enum DaemonCommand {
    Stop,
}

pub struct Repl {
    editor: Reedline,
    prompt: DefaultPrompt,
    daemon_command_tx: Option<mpsc::UnboundedSender<DaemonCommand>>,
    installer: Option<Arc<MapInstallationService>>,
}

impl Repl {
    pub fn new() -> Self {
        Self {
            editor: Reedline::create(),
            prompt: DefaultPrompt::new(DefaultPromptSegment::Empty, DefaultPromptSegment::Empty),
            daemon_command_tx: None,
            installer: None,
        }
    }

    pub fn new_with_command_tx(
        daemon_command_tx: mpsc::UnboundedSender<DaemonCommand>,
        installer: Arc<MapInstallationService>,
    ) -> Self {
        Self {
            editor: Reedline::create(),
            prompt: DefaultPrompt::new(DefaultPromptSegment::Empty, DefaultPromptSegment::Empty),
            daemon_command_tx: Some(daemon_command_tx),
            installer: Some(installer),
        }
    }

    pub async fn run(mut self) -> Result<(), String> {
        let runtime_handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            loop {
                match self.editor.read_line(&self.prompt) {
                    Ok(Signal::Success(input)) => {
                        let cmd = input.trim();
                        let mut parts = cmd.split_whitespace();
                        let command = parts.next().unwrap_or("");
                        let args: Vec<&str> = parts.collect();

                        match command {
                            "h" | "help" => {
                                self.print_help();
                            }
                            "ls" | "list" | "maps" => {
                                self.handle_list_maps(&runtime_handle);
                            }
                            "i" | "install" => {
                                self.handle_install(&runtime_handle, &args);
                            }
                            "rm" | "remove" | "uninstall" => {
                                self.handle_remove(&runtime_handle, args.first().copied());
                            }
                            "scan" | "discover" | "d" => {
                                self.handle_discover(&runtime_handle, &args);
                            }
                            "u" | "update" => {
                                self.handle_update(&runtime_handle, &args);
                            }
                            "compact" => {
                                self.handle_compact(&runtime_handle);
                            }
                            "info" => {
                                self.handle_info(&runtime_handle, args.first().copied());
                            }
                            "modify" => {
                                self.handle_modify(&runtime_handle, &args);
                            }
                            "q" | "quit" | "exit" => {
                                println!("Exiting REPL...");
                                break;
                            }
                            "S" | "stop" => {
                                match &self.daemon_command_tx {
                                    Some(tx) => {
                                        if let Err(err) = tx.send(DaemonCommand::Stop) {
                                            eprintln!("Failed to request daemon stop: {err}");
                                        } else {
                                            println!("Stop requested. Closing REPL...");
                                        }
                                    }
                                    None => eprintln!("Daemon command channel unavailable."),
                                }
                                break;
                            }
                            "" => {}
                            other => {
                                println!(
                                    "Unknown command: {other}. Type 'help' for available commands."
                                );
                            }
                        }
                    }
                    Ok(Signal::CtrlC) => {
                        println!("Interrupted (Ctrl+C). Type 'q' / 'quit' to exit.");
                    }
                    Ok(Signal::CtrlD) => {
                        println!("EOF received. Exiting...");
                        break;
                    }
                    Err(err) => {
                        eprintln!("Error reading line: {err}");
                        break;
                    }
                }
            }

            Ok::<(), String>(())
        })
        .await
        .map_err(|e| format!("REPL task join error: {e}"))?
    }

    fn print_help(&self) {
        println!("Available commands:");
        println!("  h, help - Show this help message");
        println!("  ls, list, maps - List installed maps");
        println!("  i, install <url|workshop_id> [name] - Install a map");
        println!("  rm, remove, uninstall <id> - Remove map by ID");
        println!("  u, update [id] [--check] [--force] - Check or re-download outdated Steam Workshop maps");
        println!("  scan, discover, d [u|U] - Local addons scan (d u = refresh metadata only)");
        println!("  compact - Remove orphaned records, sort by name, reindex IDs from 1");
        println!("  info <id> - Show all stored fields for a map");
        println!("  modify <id> <field> <value> - Edit a field (name, source_url, version, source_kind, workshop_id)");
        println!("  q, quit, exit - Exit the REPL");
        println!("  S, stop - Stop the daemon");
    }

    fn handle_list_maps(&self, runtime_handle: &tokio::runtime::Handle) {
        let Some(installer) = self.installer.as_ref() else {
            eprintln!("Map installer unavailable.");
            return;
        };

        match runtime_handle.block_on(installer.registry().list_maps()) {
            Ok(maps) => {
                if maps.is_empty() {
                    println!("No maps installed.");
                    return;
                }

                println!("Installed maps:");
                for map in maps {
                    self.print_map_entry(&map);
                }
            }
            Err(err) => {
                eprintln!("Failed to list maps: {err}");
            }
        }
    }

    fn handle_install(&self, runtime_handle: &tokio::runtime::Handle, args: &[&str]) {
        let Some(installer) = self.installer.as_ref() else {
            eprintln!("Map installer unavailable.");
            return;
        };

        if args.is_empty() {
            println!("Usage: install <url|workshop_id> [name]");
            return;
        }

        let source = args[0];
        let provided_name = if args.len() > 1 {
            Some(args[1..].join(" "))
        } else {
            None
        };

        let result = if source.chars().all(|ch| ch.is_ascii_digit()) {
            match source.parse::<u64>() {
                Ok(workshop_id) => {
                    runtime_handle.block_on(installer.install_from_workshop_id(workshop_id, provided_name))
                }
                Err(err) => {
                    eprintln!("Invalid workshop ID '{source}': {err}");
                    return;
                }
            }
        } else {
            runtime_handle.block_on(installer.install_from_url(source.to_string(), provided_name))
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

    fn handle_remove(&self, runtime_handle: &tokio::runtime::Handle, id_arg: Option<&str>) {
        let Some(installer) = self.installer.as_ref() else {
            eprintln!("Map installer unavailable.");
            return;
        };

        let Some(id_raw) = id_arg else {
            println!("Usage: remove <id>");
            return;
        };

        let map_id = match id_raw.parse::<u64>() {
            Ok(map_id) => map_id,
            Err(err) => {
                eprintln!("Invalid map ID '{id_raw}': {err}");
                return;
            }
        };

        match runtime_handle.block_on(installer.uninstall_map(map_id)) {
            Ok(()) => println!("Removed map #{map_id}."),
            Err(err) => eprintln!("Failed to remove map #{map_id}: {err}"),
        }
    }

    fn handle_discover(&self, runtime_handle: &tokio::runtime::Handle, args: &[&str]) {
        let Some(installer) = self.installer.as_ref() else {
            eprintln!("Map installer unavailable.");
            return;
        };

        let mode = match args.first().copied() {
            None => DiscoveryMode::Add,
            Some("u") | Some("update") => DiscoveryMode::Update,
            Some("U") | Some("forceupdate") => DiscoveryMode::ForceUpdate,
            Some(other) => {
                eprintln!(
                    "Unknown discovery argument '{other}'. Usage: d [u|U] (u=update, U=force update)."
                );
                return;
            }
        };

        match runtime_handle.block_on(installer.discover_maps(mode)) {
            Ok(report) => self.print_discovery_report(report),
            Err(err) => {
                eprintln!("Discovery failed: {err}");
            }
        }
    }

    fn handle_compact(&self, runtime_handle: &tokio::runtime::Handle) {
        let Some(installer) = self.installer.as_ref() else {
            eprintln!("Map installer unavailable.");
            return;
        };

        match runtime_handle.block_on(installer.compact_registry()) {
            Ok(report) => self.print_compact_report(report),
            Err(err) => {
                eprintln!("Compact failed: {err}");
            }
        }
    }

    fn print_compact_report(&self, report: CompactReport) {
        let kept_count = report.kept.len();
        let id_range = if kept_count == 0 {
            "none".to_string()
        } else if kept_count == 1 {
            "1".to_string()
        } else {
            format!("1–{kept_count}")
        };

        println!(
            "Compact complete: {} orphaned record(s) removed, {} map(s) reindexed (IDs {id_range}).",
            report.removed.len(),
            kept_count
        );

        if !report.removed.is_empty() {
            println!("Removed orphaned records:");
            for map in &report.removed {
                self.print_map_entry(map);
            }
        }
    }

    fn print_discovery_report(&self, report: DiscoveryReport) {
        println!(
            "Discovery complete: {} added, {} updated, {} already current, {} failed.",
            report.added.len(),
            report.updated.len(),
            report.skipped,
            report.failed
        );
        if report.added.is_empty() {
            println!("No new maps found.");
        } else {
            println!("Newly registered maps:");
            for map in report.added {
                self.print_map_entry(&map);
            }
        }
        if !report.updated.is_empty() {
            println!("Updated maps:");
            for map in report.updated {
                self.print_map_entry(&map);
            }
        }
    }

    fn print_map_entry(&self, map: &MapEntry) {
        let source_kind = match map.source_kind {
            SourceKind::Workshop => "workshop",
            SourceKind::SirPlease => "sirplease",
            SourceKind::Other => "other",
        };
        let version = map.version.as_deref().unwrap_or("-");
        let source = match map.workshop_id {
            Some(id) => format!("workshop:{id}"),
            None => map.source_url.clone(),
        };

        println!(
            "  #{} | {} | version={} | source={} ({}) | path={}",
            map.id, map.name, version, source_kind, source, map.installed_path
        );
        if let Some(updated_at) = map.workshop_updated_at {
            println!("    workshop_updated_at: {updated_at}");
        }
    }

    fn handle_update(&self, runtime_handle: &tokio::runtime::Handle, args: &[&str]) {
        let Some(installer) = self.installer.as_ref() else {
            eprintln!("Map installer unavailable.");
            return;
        };

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
                    println!("Usage: update [id] [--check] [--force]");
                    return;
                }
                map_id = Some(id);
            } else {
                println!("Usage: update [id] [--check] [--force]");
                return;
            }
        }

        match runtime_handle.block_on(installer.update_workshop_maps(map_id, force, check_only)) {
            Ok(report) => {
                if check_only {
                    self.print_workshop_check_report(&report);
                } else {
                    self.print_workshop_update_report(&report);
                }
            }
            Err(err) => eprintln!("Workshop update failed: {err}"),
        }
    }

    fn print_workshop_check_report(&self, report: &WorkshopUpdateReport) {
        println!(
            "Workshop check: {} update(s) available, {} up to date, {} failed, {} not workshop-linked",
            report.available.len(),
            report.skipped,
            report.failed.len(),
            report.not_workshop
        );

        for item in &report.available {
            let local = item
                .map
                .workshop_updated_at
                .map(|ts| ts.to_string())
                .unwrap_or_else(|| "-".to_string());
            println!(
                "  #{} | {} | workshop:{} | steam:{} | local:{}",
                item.map.id,
                item.map.name,
                item.workshop_id,
                item.steam_updated_at,
                local
            );
        }

        for (map_id, error) in &report.failed {
            eprintln!("  Failed #{}: {error}", map_id);
        }
    }

    fn print_workshop_update_report(&self, report: &WorkshopUpdateReport) {
        println!(
            "Workshop update complete: {} updated, {} skipped, {} failed, {} not workshop-linked",
            report.updated.len(),
            report.skipped,
            report.failed.len(),
            report.not_workshop
        );

        for map in &report.updated {
            println!(
                "  Updated #{}: {} ({})",
                map.id, map.name, map.installed_path
            );
        }

        for (map_id, error) in &report.failed {
            eprintln!("  Failed #{}: {error}", map_id);
        }
    }

    fn handle_info(&self, runtime_handle: &tokio::runtime::Handle, id_arg: Option<&str>) {
        let Some(installer) = self.installer.as_ref() else {
            eprintln!("Map installer unavailable.");
            return;
        };

        let Some(id_raw) = id_arg else {
            println!("Usage: info <id>");
            return;
        };

        let map_id = match id_raw.parse::<u64>() {
            Ok(map_id) => map_id,
            Err(err) => {
                eprintln!("Invalid map ID '{id_raw}': {err}");
                return;
            }
        };

        match runtime_handle.block_on(installer.registry().get_map(map_id)) {
            Ok(Some(map)) => self.print_map_detail(&map),
            Ok(None) => println!("Map #{map_id} not found."),
            Err(err) => eprintln!("Failed to load map #{map_id}: {err}"),
        }
    }

    fn handle_modify(&self, runtime_handle: &tokio::runtime::Handle, args: &[&str]) {
        let Some(installer) = self.installer.as_ref() else {
            eprintln!("Map installer unavailable.");
            return;
        };

        if args.len() < 3 {
            println!(
                "Usage: modify <id> <field> <value>\nEditable fields: name, source_url, version, source_kind, workshop_id"
            );
            return;
        }

        let map_id = match args[0].parse::<u64>() {
            Ok(map_id) => map_id,
            Err(err) => {
                eprintln!("Invalid map ID '{}': {err}", args[0]);
                return;
            }
        };

        let field = args[1];
        let value = args[2..].join(" ");

        match runtime_handle.block_on(installer.modify_map_field(map_id, field, &value)) {
            Ok(map) => {
                println!("Updated map #{}:", map.id);
                self.print_map_detail(&map);
            }
            Err(err) => eprintln!("Modify failed: {err}"),
        }
    }

    fn print_map_detail(&self, map: &MapEntry) {
        let source_kind = match map.source_kind {
            SourceKind::Workshop => "workshop",
            SourceKind::SirPlease => "sirplease",
            SourceKind::Other => "other",
        };
        let workshop_id = map
            .workshop_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "-".to_string());
        let version = map.version.as_deref().unwrap_or("-");
        let checksum = map.checksum.as_deref().unwrap_or("-");
        let checksum_kind = map.checksum_kind.as_deref().unwrap_or("-");

        println!("  id:             {}", map.id);
        println!("  name:           {}", map.name);
        println!("  source_kind:    {source_kind}");
        println!("  workshop_id:    {workshop_id}");
        println!("  source_url:     {}", map.source_url);
        println!("  version:        {version}");
        println!("  installed_path: {}", map.installed_path);
        println!("  installed_at:   {}", map.installed_at);
        if let Some(updated_at) = map.workshop_updated_at {
            println!("  workshop_updated_at: {updated_at}");
        }
        println!("  checksum:       {checksum}");
        println!("  checksum_kind:  {checksum_kind}");
    }
}

impl Default for Repl {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn start_key_listener(
    daemon_command_tx: mpsc::UnboundedSender<DaemonCommand>,
    installer: Arc<MapInstallationService>,
) -> Result<(), String> {
    println!("Press 'C' to open the REPL console");

    loop {
        let key_detected = tokio::task::spawn_blocking(move || {
            loop {
                if let Ok(true) = event::poll(std::time::Duration::from_millis(100))
                    && let Ok(Event::Key(key_event)) = event::read()
                {
                    if key_event.kind == KeyEventKind::Press {
                        match key_event.code {
                            KeyCode::Char('c') | KeyCode::Char('C') => {
                                return true;
                            }
                            _ => {}
                        }
                    }
                }
            }
        })
        .await
        .map_err(|e| format!("Key listener task join error: {e}"))?;

        if key_detected {
            println!("\nOpening REPL console... (Type 'help' for available commands or 'quit' to close)");
            let repl = Repl::new_with_command_tx(daemon_command_tx.clone(), Arc::clone(&installer));
            if let Err(err) = repl.run().await {
                eprintln!("REPL error: {err}");
            }
            println!("REPL closed. Press 'C' to open again.");
        }
    }
}
