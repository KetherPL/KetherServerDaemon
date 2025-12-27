// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use std::path::PathBuf;
use tokio::sync::mpsc::Receiver;

#[derive(Debug, Clone)]
pub enum WatcherEvent {
    Create(PathBuf),
    Remove(PathBuf),
    Modify(PathBuf),
}

#[async_trait]
pub trait Watcher: Send + Sync {
    /// Start watching the specified directory and return a receiver for events
    async fn watch(&mut self, path: PathBuf) -> anyhow::Result<Receiver<WatcherEvent>>;
    
    /// Stop watching
    async fn stop(&mut self) -> anyhow::Result<()>;
}

