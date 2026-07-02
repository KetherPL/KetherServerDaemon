// SPDX-License-Identifier: GPL-3.0-only
use std::future::Future;
use std::sync::Arc;

use crate::map_installer::MapInstallationService;

pub fn require_installer(
    installer: &Option<Arc<MapInstallationService>>,
) -> Option<&Arc<MapInstallationService>> {
    let Some(installer) = installer.as_ref() else {
        eprintln!("Map installer unavailable.");
        return None;
    };
    Some(installer)
}

pub fn block_on_installer<T>(
    handle: &tokio::runtime::Handle,
    _installer: &Arc<MapInstallationService>,
    fut: impl Future<Output = T>,
) -> T {
    handle.block_on(fut)
}
