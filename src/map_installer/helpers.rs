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

/// True when `path` is a `.vpk` under addons root or the `workshop/` subtree.
pub fn is_watched_map_path(addons_dir: &Path, path: &Path) -> bool {
    let relative = match path.strip_prefix(addons_dir) {
        Ok(relative) => relative,
        Err(_) => return false,
    };

    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_none_or(|ext| !ext.eq_ignore_ascii_case("vpk"))
    {
        return false;
    }

    use std::path::Component;

    match relative.components().collect::<Vec<_>>().as_slice() {
        [Component::Normal(_)] => true,
        [Component::Normal(first), ..] => *first == "workshop",
        _ => false,
    }
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
    use std::path::PathBuf;

    fn addons_dir() -> PathBuf {
        PathBuf::from("/home/steam/l4d2/left4dead2/addons")
    }

    #[test]
    fn is_watched_map_path_accepts_root_vpk() {
        let addons = addons_dir();
        assert!(is_watched_map_path(
            &addons,
            &addons.join("carriedoff.vpk")
        ));
    }

    #[test]
    fn is_watched_map_path_accepts_workshop_vpk() {
        let addons = addons_dir();
        assert!(is_watched_map_path(
            &addons,
            &addons.join("workshop/123456789.vpk")
        ));
        assert!(is_watched_map_path(
            &addons,
            &addons.join("workshop/nested/map.vpk")
        ));
    }

    #[test]
    fn is_watched_map_path_rejects_sourcemod_and_other_subdirs() {
        let addons = addons_dir();
        assert!(!is_watched_map_path(
            &addons,
            &addons.join("sourcemod/data/sqlite/clientprefs-sqlite.sq3-journal")
        ));
        assert!(!is_watched_map_path(
            &addons,
            &addons.join("sourcemod/plugins/foo.vpk")
        ));
        assert!(!is_watched_map_path(
            &addons,
            &addons.join("otherdir/map.vpk")
        ));
    }

    #[test]
    fn is_watched_map_path_rejects_non_vpk_extension() {
        let addons = addons_dir();
        assert!(!is_watched_map_path(&addons, &addons.join("map.zip")));
        assert!(!is_watched_map_path(
            &addons,
            &addons.join("workshop/readme.txt")
        ));
    }

    #[test]
    fn is_watched_map_path_rejects_outside_addons() {
        let addons = addons_dir();
        assert!(!is_watched_map_path(&addons, Path::new("/tmp/map.vpk")));
    }

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
