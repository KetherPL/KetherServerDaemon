// SPDX-License-Identifier: GPL-3.0-only
use std::sync::{Arc, RwLock};

use crate::config::model::Config;

pub type ConfigHandle = Arc<RwLock<Arc<Config>>>;

/// Create a shared config handle from an initial snapshot.
pub fn init_handle(config: Config) -> ConfigHandle {
    Arc::new(RwLock::new(Arc::new(config)))
}

/// Read the current config snapshot, tolerating a poisoned lock.
pub fn read_config(handle: &ConfigHandle) -> Arc<Config> {
    match handle.read() {
        Ok(guard) => guard.clone(),
        Err(e) => e.into_inner().clone(),
    }
}
