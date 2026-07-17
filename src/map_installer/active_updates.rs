// SPDX-License-Identifier: GPL-3.0-only
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use crate::registry::SourceKind;

/// Map currently being downloaded/replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveMapUpdate {
    pub name: String,
    pub map_id: u64,
    pub source_kind: SourceKind,
}

#[derive(Debug, Clone)]
struct ActiveEntry {
    update: ActiveMapUpdate,
    refs: u32,
}

/// In-memory set of maps with an update in progress.
#[derive(Debug, Clone, Default)]
pub struct ActiveUpdatesState {
    inner: Arc<RwLock<Vec<ActiveEntry>>>,
}

impl ActiveUpdatesState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment refcount for `map_id`, or insert with refs=1.
    pub fn mark_started(&self, update: ActiveMapUpdate) {
        let mut guard = self.inner.write().expect("active updates lock poisoned");
        if let Some(entry) = guard.iter_mut().find(|e| e.update.map_id == update.map_id) {
            entry.refs = entry.refs.saturating_add(1);
            entry.update = update;
            return;
        }
        guard.push(ActiveEntry { update, refs: 1 });
    }

    /// Atomically start tracking if not already active. Returns false if already in progress.
    pub fn try_mark_started(&self, update: ActiveMapUpdate) -> bool {
        let mut guard = self.inner.write().expect("active updates lock poisoned");
        if guard.iter().any(|e| e.update.map_id == update.map_id) {
            return false;
        }
        guard.push(ActiveEntry { update, refs: 1 });
        true
    }

    pub fn mark_finished(&self, map_id: u64) {
        let mut guard = self.inner.write().expect("active updates lock poisoned");
        if let Some(pos) = guard.iter().position(|e| e.update.map_id == map_id) {
            let refs = guard[pos].refs.saturating_sub(1);
            if refs == 0 {
                guard.remove(pos);
            } else {
                guard[pos].refs = refs;
            }
        }
    }

    /// Remove a map from the active set regardless of refcount (e.g. uninstall).
    pub fn clear(&self, map_id: u64) {
        let mut guard = self.inner.write().expect("active updates lock poisoned");
        guard.retain(|e| e.update.map_id != map_id);
    }

    pub fn is_active(&self, map_id: u64) -> bool {
        self.inner
            .read()
            .expect("active updates lock poisoned")
            .iter()
            .any(|e| e.update.map_id == map_id)
    }

    pub fn active_ids(&self) -> Vec<u64> {
        self.inner
            .read()
            .expect("active updates lock poisoned")
            .iter()
            .map(|e| e.update.map_id)
            .collect()
    }

    pub fn list(&self) -> Vec<ActiveMapUpdate> {
        self.inner
            .read()
            .expect("active updates lock poisoned")
            .iter()
            .map(|e| e.update.clone())
            .collect()
    }
}

/// RAII guard that clears an in-progress entry when dropped.
pub struct ActiveUpdateGuard {
    state: ActiveUpdatesState,
    map_id: u64,
}

impl ActiveUpdateGuard {
    /// Start tracking an active update. Returns `None` if this map is already being updated.
    pub fn try_begin(state: ActiveUpdatesState, update: ActiveMapUpdate) -> Option<Self> {
        let map_id = update.map_id;
        if !state.try_mark_started(update) {
            return None;
        }
        Some(Self { state, map_id })
    }
}

impl Drop for ActiveUpdateGuard {
    fn drop(&mut self) {
        self.state.mark_finished(self.map_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_and_finish_tracks_entries() {
        let state = ActiveUpdatesState::new();
        state.mark_started(ActiveMapUpdate {
            name: "A".to_string(),
            map_id: 1,
            source_kind: SourceKind::Workshop,
        });
        state.mark_started(ActiveMapUpdate {
            name: "B".to_string(),
            map_id: 2,
            source_kind: SourceKind::L4d2Center,
        });
        assert_eq!(state.list().len(), 2);
        state.mark_finished(1);
        let list = state.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].map_id, 2);
    }

    #[test]
    fn refcount_keeps_entry_until_last_finish() {
        let state = ActiveUpdatesState::new();
        let update = ActiveMapUpdate {
            name: "A".to_string(),
            map_id: 7,
            source_kind: SourceKind::Workshop,
        };
        state.mark_started(update.clone());
        state.mark_started(update);
        assert_eq!(state.list().len(), 1);
        state.mark_finished(7);
        assert_eq!(state.list().len(), 1);
        state.mark_finished(7);
        assert!(state.list().is_empty());
    }

    #[test]
    fn try_begin_rejects_duplicate_and_clears_on_drop() {
        let state = ActiveUpdatesState::new();
        {
            let guard = ActiveUpdateGuard::try_begin(
                state.clone(),
                ActiveMapUpdate {
                    name: "A".to_string(),
                    map_id: 7,
                    source_kind: SourceKind::Workshop,
                },
            );
            assert!(guard.is_some());
            assert_eq!(state.list().len(), 1);
            assert!(ActiveUpdateGuard::try_begin(
                state.clone(),
                ActiveMapUpdate {
                    name: "A".to_string(),
                    map_id: 7,
                    source_kind: SourceKind::Workshop,
                },
            )
            .is_none());
        }
        assert!(state.list().is_empty());
    }
}
