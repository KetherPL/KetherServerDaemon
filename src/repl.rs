// SPDX-License-Identifier: GPL-3.0-only

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use reedline::{DefaultPrompt, DefaultPromptSegment, Reedline, Signal};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy)]
pub enum DaemonCommand {
    Stop,
}

pub struct Repl {
    editor: Reedline,
    prompt: DefaultPrompt,
    daemon_command_tx: Option<mpsc::UnboundedSender<DaemonCommand>>,
}

impl Repl {
    pub fn new() -> Self {
        Self {
            editor: Reedline::create(),
            prompt: DefaultPrompt::new(DefaultPromptSegment::Empty, DefaultPromptSegment::Empty),
            daemon_command_tx: None,
        }
    }

    pub fn new_with_command_tx(daemon_command_tx: mpsc::UnboundedSender<DaemonCommand>) -> Self {
        Self {
            editor: Reedline::create(),
            prompt: DefaultPrompt::new(DefaultPromptSegment::Empty, DefaultPromptSegment::Empty),
            daemon_command_tx: Some(daemon_command_tx),
        }
    }

    pub async fn run(mut self) -> Result<(), String> {
        tokio::task::spawn_blocking(move || {
            loop {
                match self.editor.read_line(&self.prompt) {
                    Ok(Signal::Success(input)) => {
                        let cmd = input.trim();
                        match cmd {
                            "h" | "help" => {
                                println!("Available commands:");
                                println!("  h, help - Show this help message");
                                println!("  q, quit, exit - Exit the REPL");
                                println!("  S, stop - Stop the daemon");
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
}

impl Default for Repl {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn start_key_listener(
    daemon_command_tx: mpsc::UnboundedSender<DaemonCommand>,
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
            let repl = Repl::new_with_command_tx(daemon_command_tx.clone());
            if let Err(err) = repl.run().await {
                eprintln!("REPL error: {err}");
            }
            println!("REPL closed. Press 'C' to open again.");
        }
    }
}
