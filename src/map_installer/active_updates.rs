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

/// In-memory set of maps with an update in progress.
#[derive(Debug, Clone, Default)]
pub struct ActiveUpdatesState {
    inner: Arc<RwLock<Vec<ActiveMapUpdate>>>,
}

impl ActiveUpdatesState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mark_started(&self, update: ActiveMapUpdate) {
        let mut guard = self.inner.write().expect("active updates lock poisoned");
        guard.retain(|u| u.map_id != update.map_id);
        guard.push(update);
    }

    pub fn mark_finished(&self, map_id: u64) {
        let mut guard = self.inner.write().expect("active updates lock poisoned");
        guard.retain(|u| u.map_id != map_id);
    }

    pub fn list(&self) -> Vec<ActiveMapUpdate> {
        self.inner
            .read()
            .expect("active updates lock poisoned")
            .clone()
    }
}

/// RAII guard that clears an in-progress entry when dropped.
pub struct ActiveUpdateGuard {
    state: ActiveUpdatesState,
    map_id: u64,
}

impl ActiveUpdateGuard {
    pub fn new(state: ActiveUpdatesState, update: ActiveMapUpdate) -> Self {
        let map_id = update.map_id;
        state.mark_started(update);
        Self { state, map_id }
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
    fn guard_clears_on_drop() {
        let state = ActiveUpdatesState::new();
        {
            let _guard = ActiveUpdateGuard::new(
                state.clone(),
                ActiveMapUpdate {
                    name: "A".to_string(),
                    map_id: 7,
                    source_kind: SourceKind::Workshop,
                },
            );
            assert_eq!(state.list().len(), 1);
        }
        assert!(state.list().is_empty());
    }
}
