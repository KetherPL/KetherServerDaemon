// SPDX-License-Identifier: GPL-3.0-only
use std::sync::Arc;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use tokio::sync::mpsc;

use crate::map_installer::MapInstallationService;

use super::session::Repl;

pub async fn start_key_listener(
    daemon_command_tx: mpsc::UnboundedSender<super::session::DaemonCommand>,
    installer: Arc<MapInstallationService>,
    l4d2center_index_url: String,
) -> Result<(), String> {
    println!("Press 'C' to open the REPL console");

    loop {
        let key_detected = tokio::task::spawn_blocking(move || {
            loop {
                if let Ok(true) = event::poll(std::time::Duration::from_millis(100))
                    && let Ok(Event::Key(key_event)) = event::read()
                    && key_event.kind == KeyEventKind::Press
                {
                    match key_event.code {
                        KeyCode::Char('c') | KeyCode::Char('C') => {
                            return true;
                        }
                        _ => {}
                    }
                }
            }
        })
        .await
        .map_err(|e| format!("Key listener task join error: {e}"))?;

        if key_detected {
            println!(
                "\nOpening REPL console... (Type 'help' for available commands or 'quit' to close)"
            );
            let repl = Repl::new_with_command_tx(
                daemon_command_tx.clone(),
                Arc::clone(&installer),
                l4d2center_index_url.clone(),
            );
            if let Err(err) = repl.run().await {
                eprintln!("REPL error: {err}");
            }
            println!("REPL closed. Press 'C' to open again.");
        }
    }
}
