// SPDX-License-Identifier: GPL-3.0-only
mod api;
mod config;
mod downloader;
mod extractor;
mod logging;
mod registry;
mod sync;
mod watcher;

use std::sync::Arc;
use tokio::signal;
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use uuid::Uuid;

use config::Config;
use logging::setup_logging;
use registry::{MapEntry, Registry, SqliteRegistry};
use sync::{BackendSyncService, SyncService};
use watcher::{InotifyWatcher, Watcher};
use api::{ApiHandlers, HttpServer, WebSocketServer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load configuration
    let config = Config::load()?;
    
    // Initialize logging
    setup_logging(&config.log_level)?;
    
    info!("Starting KetherServerDaemon v{}", env!("CARGO_PKG_VERSION"));
    
    // Initialize registry
    let registry: Arc<dyn Registry> = Arc::new(SqliteRegistry::new(&config.registry_db_path).await?);
    info!("Registry initialized at {}", config.registry_db_path.display());
    
    // Initialize sync service
    let sync_service: Arc<dyn SyncService> = Arc::new(
        BackendSyncService::new(config.backend_api_url.clone(), config.backend_api_key.clone())?
    );
    
    // Initialize watcher
    let addons_dir = config.addons_dir();
    tokio::fs::create_dir_all(&addons_dir).await?;
    
    let mut watcher = InotifyWatcher::new();
    let watcher_events = watcher.watch(addons_dir.clone()).await?;
    
    // Create temp directory for downloads
    let temp_dir = std::env::temp_dir().join("kether-downloads");
    tokio::fs::create_dir_all(&temp_dir).await?;
    
    // Spawn tasks
    let registry_watcher = Arc::clone(&registry);
    let watcher_task = tokio::spawn(async move {
        info!("Watcher task started");
        let mut receiver = watcher_events;
        while let Some(event) = receiver.recv().await {
            match event {
                watcher::WatcherEvent::Create(path) => {
                    info!(path = %path.display(), "File created in addons directory");
                    // TODO: Detect new map installations and update registry
                }
                watcher::WatcherEvent::Remove(path) => {
                    info!(path = %path.display(), "File removed from addons directory");
                    // TODO: Detect map removals and update registry
                }
                watcher::WatcherEvent::Modify(path) => {
                    info!(path = %path.display(), "File modified in addons directory");
                }
            }
        }
    });
    
    let registry_sync = Arc::clone(&registry);
    let sync_service_clone = Arc::clone(&sync_service);
    let sync_interval = config.sync_interval_secs;
    let sync_task = tokio::spawn(async move {
        info!("Sync task started");
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(sync_interval));
        loop {
            interval.tick().await;
            
            // Fetch updates from backend
            match sync_service_clone.fetch_updates().await {
                Ok(updates) => {
                    for update in updates {
                        match update.action.as_str() {
                            "install" => {
                                info!(map_id = %update.map_id, "Backend requested map installation");
                                // TODO: Implement map installation logic
                            }
                            "uninstall" => {
                                info!(map_id = %update.map_id, "Backend requested map uninstallation");
                                if let Err(e) = registry_sync.remove_map(&update.map_id).await {
                                    error!(error = %e, map_id = %update.map_id, "Failed to uninstall map");
                                }
                            }
                            _ => {
                                warn!(action = %update.action, "Unknown sync action");
                            }
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to fetch updates from backend");
                }
            }
            
            // Push local state to backend
            match registry_sync.list_maps().await {
                Ok(maps) => {
                    if let Err(e) = sync_service_clone.sync_registry(maps).await {
                        error!(error = %e, "Failed to sync registry to backend");
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to list maps for sync");
                }
            }
        }
    });
    
    // Start HTTP server
    let registry_http = Arc::clone(&registry);
    let http_addr = config.local_api_bind;
    let http_server = HttpServer::new(registry_http, http_addr);
    let http_task = tokio::spawn(async move {
        if let Err(e) = http_server.serve().await {
            error!(error = %e, "HTTP server error");
        }
    });
    
    // Start WebSocket server (integrated with HTTP)
    // Note: For simplicity, WebSocket is handled via HTTP server routes
    // If needed, it can be separated into a different port
    
    info!("All services started. Waiting for shutdown signal...");
    
    // Wait for shutdown signal
    match signal::ctrl_c().await {
        Ok(()) => {
            info!("Received shutdown signal (Ctrl+C)");
        }
        Err(err) => {
            error!(error = %err, "Unable to listen for shutdown signal");
        }
    }
    
    // Graceful shutdown
    info!("Initiating graceful shutdown...");
    
    watcher_task.abort();
    sync_task.abort();
    http_task.abort();
    
    // Wait a bit for tasks to finish
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    info!("Shutdown complete");
    Ok(())
}
