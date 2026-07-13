// SPDX-License-Identifier: GPL-3.0-only
use crate::config::env::keys;

/// Nonexistent path so `Config::load()` never reads a workspace `config.toml`.
pub const ISOLATED_CONFIG_PATH: &str = "/tmp/kether-test-no-config.toml";

pub fn set_env_var(key: &str, value: &str) {
    unsafe {
        std::env::set_var(key, value);
    }
}

pub fn remove_env_var(key: &str) {
    unsafe {
        std::env::remove_var(key);
    }
}

pub fn clear_kether_env_vars() {
    remove_env_var(keys::CONFIG);
    remove_env_var(keys::L4D2_SERVER_DIR);
    remove_env_var(keys::REGISTRY_PATH);
    remove_env_var(keys::BACKEND_API_URL);
    remove_env_var(keys::BACKEND_API_KEY);
    remove_env_var(keys::LOCAL_API_BIND);
    remove_env_var(keys::SYNC_INTERVAL_SECS);
    remove_env_var(keys::LOG_LEVEL);
    remove_env_var(keys::MAX_DOWNLOAD_SIZE_BYTES);
    remove_env_var(keys::MAX_EXTRACTION_SIZE_BYTES);
    remove_env_var(keys::MAX_EXTRACTION_FILE_COUNT);
}

/// Run a closure with a single env var set, restoring prior value afterward.
pub fn with_env_var<F>(key: &str, value: &str, f: F)
where
    F: FnOnce(),
{
    let original = std::env::var(key).ok();
    set_env_var(key, value);
    f();
    if let Some(val) = original {
        set_env_var(key, &val);
    } else {
        remove_env_var(key);
    }
}

/// Run a closure with isolated KETHER env (isolated config path), restoring `KETHER_CONFIG` after.
pub fn with_isolated_config<F>(f: F)
where
    F: FnOnce(),
{
    let original_config = std::env::var(keys::CONFIG).ok();
    clear_kether_env_vars();
    set_env_var(keys::CONFIG, ISOLATED_CONFIG_PATH);
    f();
    if let Some(val) = original_config {
        set_env_var(keys::CONFIG, &val);
    } else {
        remove_env_var(keys::CONFIG);
    }
}
