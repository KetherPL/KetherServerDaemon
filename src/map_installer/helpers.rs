// SPDX-License-Identifier: GPL-3.0-only
use std::path::Path;

use crate::registry::models::SourceKind;

pub fn workshop_source_url(workshop_id: u64) -> String {
    format!("https://steamcommunity.com/sharedfiles/filedetails/?id={workshop_id}")
}

pub fn addons_relative_path(addons_dir: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(addons_dir)
        .ok()
        .map(|relative| relative.to_string_lossy().to_string())
}

pub fn source_kind_from_url(url: &str) -> SourceKind {
    let lower = url.to_lowercase();
    if lower.contains("l4d2center.com") {
        SourceKind::L4d2Center
    } else if lower.contains("sirplease.vercel.app") {
        SourceKind::SirPlease
    } else {
        SourceKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_kind_from_url_detects_l4d2center() {
        assert_eq!(
            source_kind_from_url("https://l4d2center.com/maps/servers/widebox1.7z"),
            SourceKind::L4d2Center
        );
    }

    #[test]
    fn source_kind_from_url_detects_sirplease() {
        assert_eq!(
            source_kind_from_url("https://sirplease.vercel.app/map.zip"),
            SourceKind::SirPlease
        );
    }

    #[test]
    fn source_kind_from_url_defaults_to_other() {
        assert_eq!(
            source_kind_from_url("https://example.com/map.zip"),
            SourceKind::Other
        );
    }
}
