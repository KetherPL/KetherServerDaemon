// SPDX-License-Identifier: GPL-3.0-only
use std::path::Path;

pub fn workshop_source_url(workshop_id: u64) -> String {
    format!("https://steamcommunity.com/sharedfiles/filedetails/?id={workshop_id}")
}

pub fn addons_relative_path(addons_dir: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(addons_dir)
        .ok()
        .map(|relative| relative.to_string_lossy().to_string())
}
