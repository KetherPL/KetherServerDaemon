// SPDX-License-Identifier: GPL-3.0-only
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::registry::SourceKind;

/// Compact pending map update for REST clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableMapUpdate {
    pub name: String,
    pub map_id: u64,
    pub source_kind: SourceKind,
    /// Present for workshop entries; used for console notifications (not exposed in API JSON).
    #[serde(default, skip_serializing)]
    pub workshop_id: Option<u64>,
}

#[derive(Debug, Default, Clone)]
struct PendingSnapshot {
    updates: Vec<AvailableMapUpdate>,
    last_checked_at: Option<DateTime<Utc>>,
}

/// In-memory snapshot of maps with updates available but not yet applied.
#[derive(Debug, Clone, Default)]
pub struct PendingUpdatesState {
    inner: Arc<RwLock<PendingSnapshot>>,
}

impl PendingUpdatesState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace all entries for `source_kind` with `items` (may be empty to clear).
    pub fn replace_for_source(&self, source_kind: SourceKind, items: Vec<AvailableMapUpdate>) {
        let mut guard = self.inner.write().expect("pending updates lock poisoned");
        guard
            .updates
            .retain(|u| u.source_kind != source_kind);
        guard.updates.extend(items);
        guard.last_checked_at = Some(Utc::now());
    }

    /// Drop pending entries whose map IDs were successfully applied.
    pub fn remove_map_ids(&self, map_ids: &[u64]) {
        if map_ids.is_empty() {
            return;
        }
        let mut guard = self.inner.write().expect("pending updates lock poisoned");
        guard
            .updates
            .retain(|u| !map_ids.contains(&u.map_id));
    }

    pub fn list(&self) -> Vec<AvailableMapUpdate> {
        self.inner
            .read()
            .expect("pending updates lock poisoned")
            .updates
            .clone()
    }

    pub fn last_checked_at(&self) -> Option<DateTime<Utc>> {
        self.inner
            .read()
            .expect("pending updates lock poisoned")
            .last_checked_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workshop(name: &str, map_id: u64, workshop_id: u64) -> AvailableMapUpdate {
        AvailableMapUpdate {
            name: name.to_string(),
            map_id,
            source_kind: SourceKind::Workshop,
            workshop_id: Some(workshop_id),
        }
    }

    fn l4d2(name: &str, map_id: u64) -> AvailableMapUpdate {
        AvailableMapUpdate {
            name: name.to_string(),
            map_id,
            source_kind: SourceKind::L4d2Center,
            workshop_id: None,
        }
    }

    #[test]
    fn replace_for_source_merges_and_clears() {
        let state = PendingUpdatesState::new();
        state.replace_for_source(
            SourceKind::Workshop,
            vec![workshop("A", 1, 100), workshop("B", 2, 200)],
        );
        state.replace_for_source(SourceKind::L4d2Center, vec![l4d2("c.vpk", 3)]);
        assert_eq!(state.list().len(), 3);

        state.replace_for_source(SourceKind::Workshop, vec![workshop("A2", 1, 100)]);
        let list = state.list();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|u| u.map_id == 1 && u.name == "A2"));
        assert!(list.iter().any(|u| u.map_id == 3));

        state.replace_for_source(SourceKind::L4d2Center, vec![]);
        let list = state.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].map_id, 1);
        assert!(state.last_checked_at().is_some());
    }

    #[test]
    fn remove_map_ids_drops_applied() {
        let state = PendingUpdatesState::new();
        state.replace_for_source(
            SourceKind::Workshop,
            vec![workshop("A", 1, 100), workshop("B", 2, 200)],
        );
        state.remove_map_ids(&[1]);
        let list = state.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].map_id, 2);
    }
}
