// SPDX-License-Identifier: GPL-3.0-only
use std::sync::Arc;

use reedline::{DefaultPrompt, DefaultPromptSegment, Reedline, Signal};
use tokio::sync::mpsc;

use crate::map_installer::MapInstallationService;

#[derive(Debug, Clone, Copy)]
pub enum DaemonCommand {
    Stop,
}

pub struct Repl {
    editor: Reedline,
    prompt: DefaultPrompt,
    pub(super) daemon_command_tx: Option<mpsc::UnboundedSender<DaemonCommand>>,
    pub(super) installer: Option<Arc<MapInstallationService>>,
    pub(super) l4d2center_index_url: String,
}

impl Repl {
    fn create_editor() -> (Reedline, DefaultPrompt) {
        (
            Reedline::create(),
            DefaultPrompt::new(DefaultPromptSegment::Empty, DefaultPromptSegment::Empty),
        )
    }

    pub fn new() -> Self {
        let (editor, prompt) = Self::create_editor();
        Self {
            editor,
            prompt,
            daemon_command_tx: None,
            installer: None,
            l4d2center_index_url: String::new(),
        }
    }

    pub fn new_with_command_tx(
        daemon_command_tx: mpsc::UnboundedSender<DaemonCommand>,
        installer: Arc<MapInstallationService>,
        l4d2center_index_url: String,
    ) -> Self {
        let (editor, prompt) = Self::create_editor();
        Self {
            editor,
            prompt,
            daemon_command_tx: Some(daemon_command_tx),
            installer: Some(installer),
            l4d2center_index_url,
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
                            "l4d2center" | "l4c" => {
                                self.handle_l4d2center(&runtime_handle, &args);
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
        println!("  l4d2center, l4c list - List L4D2Center catalog maps and install status");
        println!("  l4d2center install <name> - Install a map from the L4D2Center catalog");
        println!("  l4d2center update [id|name] [--check] [--force] - Check or update L4D2Center maps");
        println!("  q, quit, exit - Exit the REPL");
        println!("  S, stop - Stop the daemon");
    }
}

impl Default for Repl {
    fn default() -> Self {
        Self::new()
    }
}
