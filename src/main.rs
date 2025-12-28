// SPDX-License-Identifier: GPL-3.0-only
mod api;
mod config;
mod downloader;
mod extractor;
mod logging;
mod map_installer;
mod registry;
mod sync;
mod utils;
mod watcher;

#[cfg(test)]
mod test_helpers;

use std::sync::Arc;
use tokio::signal;
use tracing::{error, info, warn};

use config::Config;
use logging::setup_logging;
use registry::{Registry, SqliteRegistry};
use sync::{BackendSyncService, SyncService};
use watcher::{InotifyWatcher, Watcher};
use api::HttpServer;
use map_installer::MapInstallationService;

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
    
    // Initialize map installation service
    let installer = Arc::new(
        MapInstallationService::new(
            Arc::clone(&registry),
            addons_dir.clone(),
            temp_dir.clone(),
        )
        .await?
    );
    info!("Map installation service initialized");
    
    // Spawn tasks
    let installer_watcher = Arc::clone(&installer);
    let watcher_task = tokio::spawn(async move {
        info!("Watcher task started");
        let mut receiver = watcher_events;
        while let Some(event) = receiver.recv().await {
            match event {
                watcher::WatcherEvent::Create(path) => {
                    info!(path = %path.display(), "File created in addons directory");
                    // Detect new map installations
                    if let Err(e) = installer_watcher.detect_map_from_path(path).await {
                        warn!(error = %e, "Failed to detect map from path");
                    }
                }
                watcher::WatcherEvent::Remove(path) => {
                    info!(path = %path.display(), "File removed from addons directory");
                    // Map removal is handled by explicit uninstall - filesystem watcher
                    // may detect manual deletions, but we don't auto-remove from registry
                }
                watcher::WatcherEvent::Modify(path) => {
                    info!(path = %path.display(), "File modified in addons directory");
                }
            }
        }
    });
    
    let installer_sync = Arc::clone(&installer);
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
                                if let Some(ref map_entry) = update.map_entry {
                                    // Check if we have a workshop ID first
                                    if let Some(workshop_id) = map_entry.workshop_id {
                                        // Install from workshop ID
                                        if let Err(e) = installer_sync
                                            .install_from_workshop_id(workshop_id, None)
                                            .await
                                        {
                                            error!(error = %e, map_id = %update.map_id, workshop_id, "Failed to install workshop map");
                                        }
                                    } else {
                                        // Install from URL
                                        if let Err(e) = installer_sync
                                            .install_from_url(
                                                map_entry.source_url.clone(),
                                                Some(map_entry.name.clone()),
                                            )
                                            .await
                                        {
                                            error!(error = %e, map_id = %update.map_id, "Failed to install map from backend");
                                        }
                                    }
                                } else {
                                    warn!(map_id = %update.map_id, "Backend update missing installation details");
                                }
                            }
                            "uninstall" => {
                                info!(map_id = %update.map_id, "Backend requested map uninstallation");
                                if let Err(e) = installer_sync.uninstall_map(&update.map_id).await {
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
            match installer_sync.registry().list_maps().await {
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
    let installer_http = Arc::clone(&installer);
    let http_addr = config.local_api_bind;
    let http_server = HttpServer::new(registry_http, installer_http, http_addr);
    let http_task = tokio::spawn(async move {
        if let Err(e) = http_server.serve().await {
            error!(error = %e, "HTTP server error");
        }
    });
    
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
