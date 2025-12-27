// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher as NotifyWatcher};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use crate::watcher::traits::{Watcher, WatcherEvent};

pub struct InotifyWatcher {
    watcher: Option<RecommendedWatcher>,
    event_tx: Option<mpsc::Sender<WatcherEvent>>,
}

impl InotifyWatcher {
    pub fn new() -> Self {
        Self {
            watcher: None,
            event_tx: None,
        }
    }
}

#[async_trait]
impl Watcher for InotifyWatcher {
    async fn watch(&mut self, path: PathBuf) -> anyhow::Result<mpsc::Receiver<WatcherEvent>> {
        if !path.exists() {
            return Err(anyhow::anyhow!("Path does not exist: {}", path.display()));
        }
        
        let (tx, rx) = mpsc::channel(1024);
        self.event_tx = Some(tx.clone());
        
        let event_tx_clone = tx.clone();
        let mut watcher = RecommendedWatcher::new(
            move |event| {
                Self::handle_static_event(&event_tx_clone, event);
            },
            Config::default(),
        )?;
        
        watcher.watch(&path, RecursiveMode::Recursive)?;
        
        self.watcher = Some(watcher);
        info!(path = %path.display(), "Started watching directory");
        
        Ok(rx)
    }
    
    async fn stop(&mut self) -> anyhow::Result<()> {
        if let Some(watcher) = self.watcher.take() {
            // Note: RecommendedWatcher doesn't have an explicit stop method
            // Dropping it will stop watching
            drop(watcher);
            info!("Stopped watching directory");
        }
        Ok(())
    }
}

impl InotifyWatcher {
    fn handle_static_event(event_tx: &mpsc::Sender<WatcherEvent>, event: notify::Result<notify::Event>) {
        match event {
            Ok(event) => {
                for path in event.paths {
                    let event_type = match event.kind {
                        EventKind::Create(_) => WatcherEvent::Create(path.clone()),
                        EventKind::Remove(_) => WatcherEvent::Remove(path.clone()),
                        EventKind::Modify(_) => WatcherEvent::Modify(path.clone()),
                        _ => continue,
                    };
                    
                    if let Err(e) = event_tx.try_send(event_type) {
                        warn!(error = %e, "Failed to send watcher event, receiver may be closed");
                    }
                }
            }
            Err(e) => {
                error!(error = %e, "Notify watcher error");
            }
        }
    }
}

impl Default for InotifyWatcher {
    fn default() -> Self {
        Self::new()
    }
}

