// SPDX-License-Identifier: GPL-3.0-only

use std::collections::HashSet;

use crate::config::Config;
use crate::registry::MapEntry;

/// Workshop and internal map IDs hidden from website-facing API responses and sync.
#[derive(Debug, Clone, Default)]
pub struct Mapsdenylist {
    workshop_ids: HashSet<u64>,
    map_ids: HashSet<u64>,
}

impl Mapsdenylist {
    pub fn from_config(config: &Config) -> Self {
        Self {
            workshop_ids: config.hidden_workshop_ids.iter().copied().collect(),
            map_ids: config.hidden_map_ids.iter().copied().collect(),
        }
    }

    pub fn is_hidden(&self, entry: &MapEntry) -> bool {
        self.map_ids.contains(&entry.id)
            || entry
                .workshop_id
                .is_some_and(|id| self.workshop_ids.contains(&id))
    }

    pub fn filter_visible(&self, maps: Vec<MapEntry>) -> Vec<MapEntry> {
        maps.into_iter()
            .filter(|entry| !self.is_hidden(entry))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::models::SourceKind;
    use chrono::Utc;

    fn sample_entry(id: u64, workshop_id: Option<u64>) -> MapEntry {
        MapEntry {
            id,
            name: format!("Map {id}"),
            source_url: String::new(),
            source_kind: if workshop_id.is_some() {
                SourceKind::Workshop
            } else {
                SourceKind::Other
            },
            workshop_id,
            installed_path: format!("map_{id}.vpk"),
            installed_at: Utc::now(),
            workshop_updated_at: None,
            version: None,
            checksum: None,
            checksum_kind: None,
        }
    }

    fn denylist(workshop: &[u64], map_ids: &[u64]) -> Mapsdenylist {
        Mapsdenylist {
            workshop_ids: workshop.iter().copied().collect(),
            map_ids: map_ids.iter().copied().collect(),
        }
    }

    #[test]
    fn hides_by_workshop_id() {
        let bl = denylist(&[381419931], &[]);
        let entry = sample_entry(1, Some(381419931));
        assert!(bl.is_hidden(&entry));
    }

    #[test]
    fn hides_by_internal_map_id() {
        let bl = denylist(&[], &[42]);
        let entry = sample_entry(42, None);
        assert!(bl.is_hidden(&entry));
    }

    #[test]
    fn visible_when_not_listed() {
        let bl = denylist(&[111], &[222]);
        let entry = sample_entry(99, Some(333));
        assert!(!bl.is_hidden(&entry));
    }

    #[test]
    fn filter_visible_excludes_hidden_entries() {
        let bl = denylist(&[100], &[2]);
        let maps = vec![
            sample_entry(1, Some(100)),
            sample_entry(2, None),
            sample_entry(3, Some(200)),
        ];
        let visible = bl.filter_visible(maps);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, 3);
    }

    #[test]
    fn from_config_builds_sets() {
        let config = Config {
            hidden_workshop_ids: vec![10, 20],
            hidden_map_ids: vec![5],
            ..Config::default()
        };
        let bl = Mapsdenylist::from_config(&config);
        assert!(bl.is_hidden(&sample_entry(5, None)));
        assert!(bl.is_hidden(&sample_entry(99, Some(10))));
        assert!(!bl.is_hidden(&sample_entry(99, Some(99))));
    }
}
