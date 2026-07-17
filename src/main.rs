// SPDX-License-Identifier: GPL-3.0-only
mod api;
mod catalog;
mod config;
mod config_watch;
mod downloader;
mod extractor;
mod logging;
mod map_installer;
mod maps_denylist;
mod repl;
mod registry;
mod sync;
mod utils;
mod watcher;

#[cfg(test)]
mod test_helpers;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::signal;
use tracing::{debug, error, info, warn};

use config::{init_handle, read_config, Config};
use logging::setup_logging;
use registry::{JsonRegistry, Registry, SourceKind};
use sync::{BackendSyncService, SyncService};
use watcher::{InotifyWatcher, PendingEntry, Watcher, schedule_pending, should_force_sync};
use api::HttpServer;
use map_installer::{is_watched_map_path, MapInstallationService};
use maps_denylist::Mapsdenylist;
use repl::{DaemonCommand, start_key_listener};

const STEAM_HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60);

enum WatcherWork {
    Sync {
        path: PathBuf,
        first_seen: Instant,
        force_unstable: bool,
    },
    Remove {
        path: PathBuf,
    },
}

fn read_denylist(handle: &config::ConfigHandle) -> Mapsdenylist {
    Mapsdenylist::from_config(&read_config(handle))
}

fn notify_console_updates(source: &str, updates: &[map_installer::AvailableMapUpdate]) {
    println!("Map updates available ({source}): {}", updates.len());
    for item in updates {
        if let Some(workshop_id) = item.workshop_id {
            println!(
                "  - #{} \"{}\" (workshop {workshop_id})",
                item.map_id, item.name
            );
        } else {
            println!("  - #{} \"{}\"", item.map_id, item.name);
        }
    }
    println!("GET /api/maps/updates/available for machine-readable list.");
    info!(
        source,
        count = updates.len(),
        "Map updates available (auto-apply disabled)"
    );
}

async fn cleanup_download_temp_dir(temp_dir: &std::path::Path) {
    let Ok(mut entries) = tokio::fs::read_dir(temp_dir).await else {
        return;
    };
    let mut removed = 0u32;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let result = if path.is_dir() {
            tokio::fs::remove_dir_all(&path).await
        } else {
            tokio::fs::remove_file(&path).await
        };
        match result {
            Ok(()) => removed += 1,
            Err(error) => warn!(
                path = %path.display(),
                error = %error,
                "Failed to clean leftover download temp path"
            ),
        }
    }
    if removed > 0 {
        info!(removed, "Cleaned leftover files from download temp directory");
    }
}

fn registry_sync_fingerprint(maps: &[registry::MapEntry]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    for map in maps {
        map.id.hash(&mut hasher);
        map.name.hash(&mut hasher);
        map.installed_path.hash(&mut hasher);
        map.source_url.hash(&mut hasher);
        map.checksum.hash(&mut hasher);
        map.workshop_id.hash(&mut hasher);
        map.installed_at.timestamp().hash(&mut hasher);
    }
    hasher.finish()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load configuration
    let (config, config_path) = Config::load_with_path()?;
    config.validate()?;

    let config_handle = init_handle(config);
    let _config_watcher = config_watch::spawn_config_watcher(config_handle.clone(), config_path)
        .map_err(|e| anyhow::anyhow!(e))?;

    let config = read_config(&config_handle);
    
    // Initialize logging
    setup_logging(&config.log_level)?;
    
    info!("Starting KetherServerDaemon v{}", env!("CARGO_PKG_VERSION"));
    
    // Initialize registry
    let registry: Arc<dyn Registry> = Arc::new(JsonRegistry::new(&config.registry_path).await?);
    info!("Registry initialized at {}", config.registry_path.display());
    
    // Initialize sync service
    let sync_service: Arc<dyn SyncService> = Arc::new(
        BackendSyncService::new(config_handle.clone())?
    );
    
    // Initialize watcher
    let addons_dir = config.addons_dir();
    tokio::fs::create_dir_all(&addons_dir).await?;
    
    let mut watcher = InotifyWatcher::new();
    let watcher_events = watcher.watch(addons_dir.clone()).await?;
    
    // Create temp directory for downloads
    let temp_dir = std::env::temp_dir().join("kether-downloads");
    tokio::fs::create_dir_all(&temp_dir).await?;
    cleanup_download_temp_dir(&temp_dir).await;
    
    // Initialize map installation service
    let installer = Arc::new(
        MapInstallationService::new(
            Arc::clone(&registry),
            addons_dir.clone(),
            temp_dir.clone(),
            config.max_download_size_bytes,
            config.max_extraction_size_bytes,
            config.max_extraction_file_count,
        )
        .await?
    );
    info!("Map installation service initialized");

    let (daemon_tx, mut daemon_rx) = tokio::sync::mpsc::unbounded_channel::<DaemonCommand>();
    
    // Spawn tasks
    let installer_sync_worker = Arc::clone(&installer);
    let addons_dir_watcher = addons_dir.clone();
    let (watcher_work_tx, mut watcher_work_rx) =
        tokio::sync::mpsc::channel::<WatcherWork>(128);

    let watcher_worker = tokio::spawn(async move {
        info!("Watcher sync worker started");
        while let Some(work) = watcher_work_rx.recv().await {
            match work {
                WatcherWork::Sync {
                    path,
                    first_seen,
                    force_unstable,
                } => {
                    let pending_age_ms = Instant::now().duration_since(first_seen).as_millis();
                    if !force_unstable {
                        // Authoritative stability check lives in the worker so the
                        // event loop never sleeps or awaits MD5/registry I/O.
                        if !utils::file_is_stable(&path).await {
                            debug!(
                                path = %path.display(),
                                pending_age_ms,
                                "File unstable at worker handoff; syncing anyway after wait"
                            );
                        }
                    } else {
                        warn!(
                            path = %path.display(),
                            pending_age_ms,
                            "File never stabilized, forcing sync attempt"
                        );
                    }
                    if let Err(e) = installer_sync_worker.sync_map_from_path(path.clone()).await {
                        warn!(
                            error = %e,
                            path = %path.display(),
                            pending_age_ms,
                            forced_unstable = force_unstable,
                            "Failed to sync map from path"
                        );
                    }
                }
                WatcherWork::Remove { path } => {
                    if let Err(e) = installer_sync_worker.remove_map_by_path(path).await {
                        warn!(error = %e, "Failed to remove map from registry after file deletion");
                    }
                }
            }
        }
    });

    let watcher_task = tokio::spawn(async move {
        info!("Watcher task started");
        let mut receiver = watcher_events;
        let mut pending: HashMap<PathBuf, PendingEntry> = HashMap::new();
        let mut last_unstable_log: HashMap<PathBuf, Instant> = HashMap::new();
        let debounce_window = Duration::from_secs(1);
        let max_stable_wait = Duration::from_secs(60);
        let mut poll_tick = tokio::time::interval(Duration::from_millis(250));

        loop {
            tokio::select! {
                event = receiver.recv() => {
                    let Some(event) = event else {
                        break;
                    };

                    match event {
                        watcher::WatcherEvent::Create(path) => {
                            if is_watched_map_path(&addons_dir_watcher, &path) {
                                let is_new = !pending.contains_key(&path);
                                schedule_pending(
                                    &mut pending,
                                    &mut last_unstable_log,
                                    path.clone(),
                                    Instant::now(),
                                    debounce_window,
                                );
                                if is_new {
                                    info!(path = %path.display(), "File created in addons directory");
                                }
                            }
                        }
                        watcher::WatcherEvent::Modify(path) => {
                            if is_watched_map_path(&addons_dir_watcher, &path) {
                                let is_new = !pending.contains_key(&path);
                                schedule_pending(
                                    &mut pending,
                                    &mut last_unstable_log,
                                    path.clone(),
                                    Instant::now(),
                                    debounce_window,
                                );
                                if is_new {
                                    info!(path = %path.display(), "File modified in addons directory");
                                }
                            }
                        }
                        watcher::WatcherEvent::Remove(path) => {
                            if is_watched_map_path(&addons_dir_watcher, &path) {
                                info!(path = %path.display(), "File removed from addons directory");
                                pending.remove(&path);
                                last_unstable_log.remove(&path);
                                if watcher_work_tx
                                    .try_send(WatcherWork::Remove { path })
                                    .is_err()
                                {
                                    warn!("Watcher work queue full; dropped remove job");
                                }
                            }
                        }
                    }
                }
                _ = poll_tick.tick() => {
                    let now = Instant::now();
                    let ready: Vec<(PathBuf, PendingEntry)> = pending
                        .iter()
                        .filter(|(_, entry)| entry.deadline <= now)
                        .map(|(path, entry)| (path.clone(), *entry))
                        .collect();

                    for (path, entry) in ready {
                        let force_unstable = should_force_sync(entry.first_seen, now, max_stable_wait);

                        // Fast non-blocking stability probe: only enqueue when stable or forced.
                        // Avoid sleeping in the event loop (file_is_stable sleeps 150ms).
                        let size_now = std::fs::metadata(&path).ok().map(|m| m.len());
                        let looks_stable = size_now.is_some_and(|size| {
                            // Re-check size without sleep; worker does the authoritative stable check.
                            std::fs::metadata(&path).ok().map(|m| m.len()) == Some(size)
                        });

                        if !force_unstable && !looks_stable {
                            let should_log = last_unstable_log
                                .get(&path)
                                .is_none_or(|last| now.duration_since(*last) >= Duration::from_secs(1));
                            if should_log {
                                debug!(
                                    path = %path.display(),
                                    "File still unstable, waiting for size to settle"
                                );
                                last_unstable_log.insert(path.clone(), now);
                            }
                            continue;
                        }

                        pending.remove(&path);
                        last_unstable_log.remove(&path);
                        if watcher_work_tx
                            .try_send(WatcherWork::Sync {
                                path,
                                first_seen: entry.first_seen,
                                force_unstable,
                            })
                            .is_err()
                        {
                            warn!("Watcher work queue full; dropped sync job");
                        }
                    }
                }
            }
        }
    });
    
    let installer_sync = Arc::clone(&installer);
    let sync_service_clone = Arc::clone(&sync_service);
    let sync_config_handle = config_handle.clone();
    let sync_task = tokio::spawn(async move {
        info!("Sync task started");
        let mut interval_secs = read_config(&sync_config_handle).sync_interval_secs;
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        // Per-update exponential backoff for repeatedly failing backend actions.
        let mut failure_backoff: HashMap<String, (u32, Instant)> = HashMap::new();
        let mut last_push_fingerprint: Option<u64> = None;

        loop {
            interval.tick().await;

            let current_config = read_config(&sync_config_handle);
            if current_config.sync_interval_secs != interval_secs {
                interval_secs = current_config.sync_interval_secs;
                // Avoid an immediate burst tick when rebuilding the interval.
                interval = tokio::time::interval_at(
                    tokio::time::Instant::now() + Duration::from_secs(interval_secs),
                    Duration::from_secs(interval_secs),
                );
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            }

            // Prune stale backoff entries (keep for at most 1h past expiry).
            let backoff_cutoff = Instant::now() - Duration::from_secs(3600);
            failure_backoff.retain(|_, (_, retry_after)| *retry_after > backoff_cutoff);

            // Fetch updates from backend
            match sync_service_clone.fetch_updates().await {
                Ok(updates) => {
                    let now = Instant::now();
                    for update in updates {
                        let update_key = format!("{}:{}", update.action, update.map_id);
                        if let Some((_, retry_after)) = failure_backoff.get(&update_key)
                            && *retry_after > now
                        {
                            debug!(
                                map_id = %update.map_id,
                                action = %update.action,
                                "Skipping backend update due to backoff"
                            );
                            continue;
                        }

                        let result = match update.action.as_str() {
                            "install" => {
                                info!(map_id = %update.map_id, "Backend requested map installation");
                                if let Some(ref map_entry) = update.map_entry {
                                    if let Some(workshop_id) = map_entry.workshop_id {
                                        installer_sync
                                            .install_from_workshop_id(workshop_id, None)
                                            .await
                                            .map(|_| ())
                                    } else {
                                        installer_sync
                                            .install_from_url(
                                                map_entry.source_url.clone(),
                                                Some(map_entry.name.clone()),
                                            )
                                            .await
                                            .map(|_| ())
                                    }
                                } else {
                                    warn!(map_id = %update.map_id, "Backend update missing installation details");
                                    Ok(())
                                }
                            }
                            "uninstall" => {
                                info!(map_id = %update.map_id, "Backend requested map uninstallation");
                                match update.map_id.parse::<u64>() {
                                    Ok(map_id) => match installer_sync.uninstall_map(map_id).await {
                                        Ok(()) => Ok(()),
                                        Err(e) if e.to_string().contains("not found") => Ok(()),
                                        Err(e) => Err(e),
                                    },
                                    Err(e) => {
                                        error!(error = %e, map_id = %update.map_id, "Invalid map ID format from backend");
                                        Ok(())
                                    }
                                }
                            }
                            _ => {
                                warn!(action = %update.action, "Unknown sync action");
                                Ok(())
                            }
                        };

                        match result {
                            Ok(()) => {
                                failure_backoff.remove(&update_key);
                            }
                            Err(e) => {
                                error!(error = %e, map_id = %update.map_id, action = %update.action, "Failed to apply backend update");
                                let failures = failure_backoff
                                    .get(&update_key)
                                    .map(|(count, _)| *count)
                                    .unwrap_or(0)
                                    .saturating_add(1);
                                let delay_secs = 2_u64.saturating_pow(failures.min(6));
                                failure_backoff.insert(
                                    update_key,
                                    (failures, Instant::now() + Duration::from_secs(delay_secs)),
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to fetch updates from backend");
                }
            }

            // Push local state to backend when content changed.
            match installer_sync.registry().list_maps().await {
                Ok(maps) => {
                    let visible = read_denylist(&sync_config_handle).filter_visible(maps);
                    let fingerprint = registry_sync_fingerprint(&visible);
                    if last_push_fingerprint == Some(fingerprint) {
                        debug!("Skipping registry push; content unchanged");
                    } else if let Err(e) = sync_service_clone.sync_registry(visible).await {
                        error!(error = %e, "Failed to sync registry to backend");
                    } else {
                        last_push_fingerprint = Some(fingerprint);
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to list maps for sync");
                }
            }
        }
    });

    let installer_steam_health = Arc::clone(&installer);
    let steam_health_task = tokio::spawn(async move {
        info!("Steam connection health-check task started");
        let mut interval = tokio::time::interval(STEAM_HEALTH_CHECK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            installer_steam_health.steam_health_check().await;
        }
    });

    let installer_map_update = Arc::clone(&installer);
    let map_update_config_handle = config_handle.clone();
    let pending_updates_task = installer.pending_updates();
    let map_update_task = tokio::spawn(async move {
        info!("Periodic map update check task started");
        let mut interval_days = read_config(&map_update_config_handle).map_update_check_interval_days;
        let period = Duration::from_secs(interval_days.saturating_mul(86_400));
        // First check after one full interval so restarts do not hammer Steam/CDN.
        let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            interval.tick().await;

            let current = read_config(&map_update_config_handle);
            if current.map_update_check_interval_days != interval_days {
                interval_days = current.map_update_check_interval_days.max(1);
                let period = Duration::from_secs(interval_days.saturating_mul(86_400));
                interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            }

            if !current.workshop_update_check_enabled && !current.l4d2center_update_check_enabled {
                pending_updates_task.replace_for_source(SourceKind::Workshop, vec![]);
                pending_updates_task.replace_for_source(SourceKind::L4d2Center, vec![]);
                continue;
            }

            if current.workshop_update_check_enabled {
                match installer_map_update
                    .update_workshop_maps(None, false, true)
                    .await
                {
                    Ok(report) => {
                        let available: Vec<_> = report
                            .available
                            .iter()
                            .map(|item| map_installer::AvailableMapUpdate {
                                name: item.map.name.clone(),
                                map_id: item.map.id,
                                source_kind: SourceKind::Workshop,
                                workshop_id: Some(item.workshop_id),
                            })
                            .collect();
                        let exclude = installer_map_update.active_updates().active_ids();
                        pending_updates_task.replace_for_source_excluding(
                            SourceKind::Workshop,
                            available.clone(),
                            &exclude,
                        );

                        if !available.is_empty() {
                            if current.workshop_update_auto_apply {
                                info!(
                                    count = available.len(),
                                    "Applying available workshop map updates"
                                );
                                match installer_map_update
                                    .update_workshop_maps(None, false, false)
                                    .await
                                {
                                    Ok(apply_report) => {
                                        for failure in &apply_report.failed {
                                            error!(
                                                map_id = failure.map_id,
                                                error = %failure.error,
                                                "Failed to auto-apply workshop update"
                                            );
                                        }
                                        info!(
                                            updated = apply_report.updated.len(),
                                            failed = apply_report.failed.len(),
                                            "Workshop auto-apply finished"
                                        );
                                    }
                                    Err(e) => {
                                        error!(error = %e, "Workshop auto-apply failed");
                                    }
                                }
                            } else {
                                notify_console_updates("workshop", &available);
                            }
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Periodic workshop update check failed");
                    }
                }
            } else {
                pending_updates_task.replace_for_source(SourceKind::Workshop, vec![]);
            }

            if current.l4d2center_update_check_enabled {
                let index_url = current.l4d2center_index_url.clone();
                match installer_map_update
                    .update_l4d2center_maps(&index_url, None, None, false, true)
                    .await
                {
                    Ok(report) => {
                        let available: Vec<_> = report
                            .available
                            .iter()
                            .map(|item| map_installer::AvailableMapUpdate {
                                name: item.name.clone(),
                                map_id: item.map_id,
                                source_kind: SourceKind::L4d2Center,
                                workshop_id: None,
                            })
                            .collect();
                        let exclude = installer_map_update.active_updates().active_ids();
                        pending_updates_task.replace_for_source_excluding(
                            SourceKind::L4d2Center,
                            available.clone(),
                            &exclude,
                        );

                        if !available.is_empty() {
                            if current.l4d2center_update_auto_apply {
                                info!(
                                    count = available.len(),
                                    "Applying available L4D2Center map updates"
                                );
                                match installer_map_update
                                    .update_l4d2center_maps(&index_url, None, None, false, false)
                                    .await
                                {
                                    Ok(apply_report) => {
                                        for failure in &apply_report.failed {
                                            error!(
                                                map_id = failure.map_id,
                                                error = %failure.error,
                                                "Failed to auto-apply L4D2Center update"
                                            );
                                        }
                                        info!(
                                            updated = apply_report.updated.len(),
                                            failed = apply_report.failed.len(),
                                            "L4D2Center auto-apply finished"
                                        );
                                    }
                                    Err(e) => {
                                        error!(error = %e, "L4D2Center auto-apply failed");
                                    }
                                }
                            } else {
                                notify_console_updates("l4d2center", &available);
                            }
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Periodic L4D2Center update check failed");
                    }
                }
            } else {
                pending_updates_task.replace_for_source(SourceKind::L4d2Center, vec![]);
            }
        }
    });
    
    // Start HTTP server
    let registry_http = Arc::clone(&registry);
    let installer_http = Arc::clone(&installer);
    let http_addr = config.local_api_bind;
    let http_config_handle = config_handle.clone();
    let http_server = HttpServer::new(
        registry_http,
        installer_http,
        http_addr,
        http_config_handle,
    );
    let http_task = tokio::spawn(async move {
        if let Err(e) = http_server.serve().await {
            error!(error = %e, "HTTP server error");
        }
    });

    let repl_tx = daemon_tx.clone();
    let repl_installer = Arc::clone(&installer);
    let repl_config_handle = config_handle.clone();
    let repl_task = tokio::spawn(async move {
        if let Err(e) = start_key_listener(repl_tx, repl_installer, repl_config_handle).await {
            error!(error = %e, "REPL key listener error");
        }
    });
    
    info!("All services started. Waiting for shutdown signal...");
    
    // Wait for shutdown signal
    tokio::select! {
        res = signal::ctrl_c() => match res {
            Ok(()) => {
                info!("Received shutdown signal (Ctrl+C)");
            }
            Err(err) => {
                error!(error = %err, "Unable to listen for shutdown signal");
            }
        },
        cmd = daemon_rx.recv() => match cmd {
            Some(DaemonCommand::Stop) => {
                info!("Received stop command from REPL");
            }
            None => {
                warn!("REPL command channel closed");
            }
        },
    }
    
    // Graceful shutdown
    info!("Initiating graceful shutdown...");
    
    watcher_task.abort();
    watcher_worker.abort();
    sync_task.abort();
    steam_health_task.abort();
    map_update_task.abort();
    http_task.abort();
    repl_task.abort();
    
    // Give aborted tasks a brief moment to unwind and flush logs.
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    
    info!("Shutdown complete");
    
    // The REPL key listener runs an infinite keyboard-polling loop inside
    // `spawn_blocking`, which cannot be aborted. Dropping the runtime would block
    // forever waiting on that thread, so force process termination here.
    std::process::exit(0);
}
