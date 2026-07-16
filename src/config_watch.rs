// SPDX-License-Identifier: GPL-3.0-only

use crate::config::apply_env_overrides;
use crate::config::{Config, ConfigChange, ConfigHandle, CONF_FILE_NAME};
use notify_debouncer_full::notify::{
    EventKind, RecommendedWatcher, RecursiveMode,
    event::{AccessKind, AccessMode, ModifyKind},
};
use notify_debouncer_full::{
    DebounceEventResult, DebouncedEvent, Debouncer, RecommendedCache, new_debouncer,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// Last successfully observed on-disk identity for config.toml.
/// Used to skip redundant reloads when metadata is unchanged.
static LAST_FILE_IDENTITY: Mutex<Option<(SystemTime, u64)>> = Mutex::new(None);

fn file_identity(path: &Path) -> Result<(SystemTime, u64), String> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("Failed to stat {}: {}", path.display(), e))?;
    let modified = metadata
        .modified()
        .map_err(|e| format!("Failed to read mtime for {}: {}", path.display(), e))?;
    Ok((modified, metadata.len()))
}

fn identity_unchanged(path: &Path) -> Result<bool, String> {
    let identity = file_identity(path)?;
    Ok(LAST_FILE_IDENTITY
        .lock()
        .map_err(|e| format!("Config watcher state lock poisoned: {}", e))?
        .as_ref() == Some(&identity))
}

fn remember_identity(path: &Path) -> Result<(), String> {
    *LAST_FILE_IDENTITY
        .lock()
        .map_err(|e| format!("Config watcher state lock poisoned: {}", e))? =
        Some(file_identity(path)?);
    Ok(())
}

pub fn apply_reload(handle: &ConfigHandle, path: &Path) -> Result<ConfigChange, String> {
    if identity_unchanged(path)? {
        return Ok(ConfigChange {
            unchanged: true,
            ..ConfigChange::default()
        });
    }

    let mut new_config = {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to reload config: {}", e))?;
        Config::from_toml_str(&content).map_err(|e| format!("Failed to reload config: {}", e))?
    };

    apply_env_overrides(&mut new_config)
        .map_err(|e| format!("Failed to apply env overrides on reload: {}", e))?;

    new_config
        .validate()
        .map_err(|e| format!("Reloaded config failed validation: {}", e))?;

    let change = match handle.write() {
        Ok(mut guard) => {
            let previous = guard.clone();
            let change = previous.diff(&new_config);
            if !change.live_applied.is_empty() {
                *guard = Arc::new(previous.with_live_fields_from(&new_config));
            }
            change
        }
        Err(e) => {
            let mut guard = e.into_inner();
            let previous = guard.clone();
            let change = previous.diff(&new_config);
            if !change.live_applied.is_empty() {
                *guard = Arc::new(previous.with_live_fields_from(&new_config));
            }
            change
        }
    };

    remember_identity(path)?;
    Ok(change)
}

fn event_mentions_config(event: &DebouncedEvent, config_path: &Path) -> bool {
    let expected_name = config_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(CONF_FILE_NAME);
    event.event.paths.iter().any(|path| {
        path == config_path || path.file_name().and_then(|n| n.to_str()) == Some(expected_name)
    })
}

/// Returns true for filesystem events that indicate config content may have changed.
///
/// Ignores `Access(Open)` and `Access(Close(Read))`, which are emitted when this process
/// reads config.toml during reload and would otherwise cause a reload feedback loop.
fn is_substantive_config_event(event: &DebouncedEvent, config_path: &Path) -> bool {
    if !event_mentions_config(event, config_path) {
        return false;
    }

    match event.kind {
        EventKind::Modify(ModifyKind::Data(_)) => true,
        EventKind::Modify(ModifyKind::Name(_)) => true,
        EventKind::Create(_) => true,
        EventKind::Remove(_) => true,
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Modify(ModifyKind::Metadata(
            notify_debouncer_full::notify::event::MetadataKind::WriteTime,
        )) => true,
        _ => false,
    }
}

pub fn spawn_config_watcher(
    handle: ConfigHandle,
    config_path: PathBuf,
) -> Result<Debouncer<RecommendedWatcher, RecommendedCache>, String> {
    let watch_parent = config_path
        .parent()
        .ok_or_else(|| "Config path has no parent directory".to_string())?
        .to_path_buf();
    let watched_path = config_path.clone();

    // Seed identity so the first debounced noise does not reload immediately.
    if let Err(err) = remember_identity(&watched_path) {
        eprintln!("Config watcher: failed to seed file identity: {}", err);
    }

    let mut debouncer = new_debouncer(
        Duration::from_secs(1),
        None,
        move |result: DebounceEventResult| match result {
            Ok(events) => {
                let relevant = events
                    .iter()
                    .any(|event| is_substantive_config_event(event, &watched_path));
                if !relevant {
                    return;
                }

                match apply_reload(&handle, &watched_path) {
                    Ok(change) => change.log(),
                    Err(err) => eprintln!("Config hot reload failed: {}", err),
                }
            }
            Err(errors) => {
                for err in errors {
                    eprintln!("Config watcher error: {}", err);
                }
            }
        },
    )
    .map_err(|e| format!("Failed to create config watcher: {}", e))?;

    debouncer
        .watch(&watch_parent, RecursiveMode::NonRecursive)
        .map_err(|e| format!("Failed to watch config directory {}: {}", watch_parent.display(), e))?;

    Ok(debouncer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::init_handle;
    use notify_debouncer_full::notify::Event;
    use std::net::SocketAddr;
    use std::path::PathBuf;
    use std::str::FromStr;

    fn write_config(path: &Path, content: &str) {
        std::fs::write(path, content).expect("failed to write config fixture");
    }

    fn base_toml(hidden_workshop: &str, sync_interval: u64) -> String {
        format!(
            r#"
l4d2_server_dir = "/home/steam/l4d2"
registry_path = "registry.json"
backend_api_url = "http://127.0.0.1:3001/api"
local_api_bind = "127.0.0.1:8080"
sync_interval_secs = {sync_interval}
log_level = "info"
hidden_workshop_ids = [{hidden_workshop}]
hidden_map_ids = []
"#
        )
    }

    fn reset_identity_cache() {
        *LAST_FILE_IDENTITY.lock().expect("identity lock") = None;
    }

    #[test]
    fn apply_reload_updates_snapshot_and_returns_diff() {
        reset_identity_cache();
        let tmp = tempfile::tempdir().expect("tempdir");
        let config_path = tmp.path().join(CONF_FILE_NAME);
        write_config(&config_path, &base_toml("", 300));

        let initial = Config::load_from(&config_path).expect("initial load");
        let handle = init_handle(initial);

        write_config(&config_path, &base_toml("381419931", 300));
        let change = apply_reload(&handle, &config_path).expect("apply reload");
        assert!(!change.unchanged);
        assert!(change.live_applied.contains(&"hidden_workshop_ids"));
        assert!(change.requires_restart.is_empty());

        let snapshot = handle.read().expect("read lock").clone();
        assert_eq!(snapshot.hidden_workshop_ids, vec![381419931]);
    }

    #[test]
    fn apply_reload_keeps_old_snapshot_on_invalid_toml() {
        reset_identity_cache();
        let tmp = tempfile::tempdir().expect("tempdir");
        let config_path = tmp.path().join(CONF_FILE_NAME);
        write_config(&config_path, &base_toml("", 300));

        let initial = Config::load_from(&config_path).expect("initial load");
        let handle = init_handle(initial);
        remember_identity(&config_path).expect("seed identity");

        write_config(&config_path, "not [valid");
        let result = apply_reload(&handle, &config_path);
        assert!(result.is_err());

        let snapshot = handle.read().expect("read lock").clone();
        assert!(snapshot.hidden_workshop_ids.is_empty());
    }

    #[test]
    fn apply_reload_keeps_old_snapshot_on_invalid_config() {
        reset_identity_cache();
        let tmp = tempfile::tempdir().expect("tempdir");
        let config_path = tmp.path().join(CONF_FILE_NAME);
        write_config(&config_path, &base_toml("", 300));

        let initial = Config::load_from(&config_path).expect("initial load");
        let handle = init_handle(initial);
        remember_identity(&config_path).expect("seed identity");

        write_config(&config_path, &base_toml("", 0));
        let result = apply_reload(&handle, &config_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("validation"));

        let snapshot = handle.read().expect("read lock").clone();
        assert_eq!(snapshot.sync_interval_secs, 300);
    }

    #[test]
    fn apply_reload_preserves_old_arc_snapshots() {
        reset_identity_cache();
        let tmp = tempfile::tempdir().expect("tempdir");
        let config_path = tmp.path().join(CONF_FILE_NAME);
        write_config(&config_path, &base_toml("", 300));

        let initial = Config::load_from(&config_path).expect("initial load");
        let handle = init_handle(initial);
        let old_snapshot = handle.read().expect("read lock").clone();

        write_config(
            &config_path,
            r#"
l4d2_server_dir = "/other/path"
registry_path = "registry.json"
backend_api_url = "http://127.0.0.1:3001/api"
local_api_bind = "127.0.0.1:8080"
sync_interval_secs = 600
log_level = "info"
hidden_workshop_ids = []
hidden_map_ids = []
"#,
        );
        let change = apply_reload(&handle, &config_path).expect("apply reload");
        assert!(change.live_applied.contains(&"sync_interval_secs"));
        assert!(change.requires_restart.contains(&"l4d2_server_dir"));

        assert_eq!(old_snapshot.sync_interval_secs, 300);
        assert_eq!(
            old_snapshot.l4d2_server_dir,
            PathBuf::from("/home/steam/l4d2")
        );

        let new_snapshot = handle.read().expect("read lock").clone();
        assert_eq!(new_snapshot.sync_interval_secs, 600);
        // Restart-required fields must stay at the previously live values.
        assert_eq!(
            new_snapshot.l4d2_server_dir,
            PathBuf::from("/home/steam/l4d2")
        );
    }

    #[test]
    fn apply_reload_skips_when_file_identity_unchanged() {
        reset_identity_cache();
        let tmp = tempfile::tempdir().expect("tempdir");
        let config_path = tmp.path().join(CONF_FILE_NAME);
        write_config(&config_path, &base_toml("", 300));

        let initial = Config::load_from(&config_path).expect("initial load");
        let handle = init_handle(initial);
        remember_identity(&config_path).expect("seed identity");

        let change = apply_reload(&handle, &config_path).expect("second apply");
        assert!(change.unchanged);
    }

    #[test]
    fn substantive_event_filter_ignores_open_and_close_read() {
        let config_path = PathBuf::from("/tmp/config.toml");
        let open = DebouncedEvent::new(
            Event::new(EventKind::Access(AccessKind::Open(AccessMode::Read)))
                .add_path(config_path.clone()),
            std::time::Instant::now(),
        );
        let close_read = DebouncedEvent::new(
            Event::new(EventKind::Access(AccessKind::Close(AccessMode::Read)))
                .add_path(config_path.clone()),
            std::time::Instant::now(),
        );
        let modify = DebouncedEvent::new(
            Event::new(EventKind::Modify(ModifyKind::Data(
                notify_debouncer_full::notify::event::DataChange::Any,
            )))
            .add_path(config_path),
            std::time::Instant::now(),
        );

        assert!(!is_substantive_config_event(&open, &PathBuf::from("/tmp/config.toml")));
        assert!(!is_substantive_config_event(
            &close_read,
            &PathBuf::from("/tmp/config.toml")
        ));
        assert!(is_substantive_config_event(
            &modify,
            &PathBuf::from("/tmp/config.toml")
        ));
    }

    #[test]
    fn watcher_applies_changes_end_to_end() {
        reset_identity_cache();
        let tmp = tempfile::tempdir().expect("tempdir");
        let config_path = tmp.path().join(CONF_FILE_NAME);
        write_config(&config_path, &base_toml("", 300));

        let initial = Config::load_from(&config_path).expect("initial load");
        let handle = init_handle(initial);

        let _watcher =
            spawn_config_watcher(handle.clone(), config_path.clone()).expect("watcher start");
        std::thread::sleep(Duration::from_millis(200));
        write_config(&config_path, &base_toml("999", 300));

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let snapshot = handle.read().expect("read lock").clone();
            if snapshot.hidden_workshop_ids == vec![999] {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "watcher did not apply update in time"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn diff_classifies_live_and_restart_fields() {
        let old = Config::default();
        let mut new = Config::default();
        new.hidden_workshop_ids = vec![1];
        new.sync_interval_secs = 120;
        new.local_api_bind = SocketAddr::from_str("127.0.0.1:9090").unwrap();

        let change = old.diff(&new);
        assert!(change.live_applied.contains(&"hidden_workshop_ids"));
        assert!(change.live_applied.contains(&"sync_interval_secs"));
        assert!(change.requires_restart.contains(&"local_api_bind"));
    }
}
