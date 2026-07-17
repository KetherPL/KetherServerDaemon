// SPDX-License-Identifier: GPL-3.0-only
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use crate::registry::SourceKind;

/// High-level phase of an in-flight map update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    #[default]
    Downloading,
    Extracting,
    Installing,
}

/// Live progress fields for an active map update.
#[derive(Debug, Clone, Default)]
pub struct UpdateProgressPatch {
    pub phase: Option<UpdatePhase>,
    pub bytes_downloaded: Option<u64>,
    pub bytes_total: Option<Option<u64>>,
    pub detail: Option<Option<String>>,
}

/// Map currently being downloaded/replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveMapUpdate {
    pub name: String,
    pub map_id: u64,
    pub source_kind: SourceKind,
    pub phase: UpdatePhase,
    pub bytes_downloaded: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ActiveMapUpdate {
    pub fn new(name: String, map_id: u64, source_kind: SourceKind) -> Self {
        Self {
            name,
            map_id,
            source_kind,
            phase: UpdatePhase::Downloading,
            bytes_downloaded: 0,
            bytes_total: None,
            percent: None,
            detail: None,
        }
    }

    fn recompute_percent(&mut self) {
        self.percent = match self.bytes_total {
            Some(total) if total > 0 => {
                let pct = ((self.bytes_downloaded as f64 / total as f64) * 100.0).floor() as u64;
                Some(pct.min(100) as u8)
            }
            _ => None,
        };
    }

    fn apply_progress(&mut self, patch: UpdateProgressPatch) {
        if let Some(phase) = patch.phase {
            self.phase = phase;
        }
        if let Some(downloaded) = patch.bytes_downloaded {
            self.bytes_downloaded = downloaded;
        }
        if let Some(total) = patch.bytes_total {
            self.bytes_total = total;
        }
        if let Some(detail) = patch.detail {
            self.detail = detail;
        }
        self.recompute_percent();
    }
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

    /// Update progress fields for an active map (no refcount change).
    pub fn set_progress(&self, map_id: u64, patch: UpdateProgressPatch) {
        let mut guard = self.inner.write().expect("active updates lock poisoned");
        if let Some(entry) = guard.iter_mut().find(|e| e.update.map_id == map_id) {
            entry.update.apply_progress(patch);
        }
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
        state.mark_started(ActiveMapUpdate::new(
            "A".to_string(),
            1,
            SourceKind::Workshop,
        ));
        state.mark_started(ActiveMapUpdate::new(
            "B".to_string(),
            2,
            SourceKind::L4d2Center,
        ));
        assert_eq!(state.list().len(), 2);
        state.mark_finished(1);
        let list = state.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].map_id, 2);
    }

    #[test]
    fn refcount_keeps_entry_until_last_finish() {
        let state = ActiveUpdatesState::new();
        let update = ActiveMapUpdate::new("A".to_string(), 7, SourceKind::Workshop);
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
                ActiveMapUpdate::new("A".to_string(), 7, SourceKind::Workshop),
            );
            assert!(guard.is_some());
            assert_eq!(state.list().len(), 1);
            assert!(ActiveUpdateGuard::try_begin(
                state.clone(),
                ActiveMapUpdate::new("A".to_string(), 7, SourceKind::Workshop),
            )
            .is_none());
        }
        assert!(state.list().is_empty());
    }

    #[test]
    fn set_progress_updates_bytes_and_percent() {
        let state = ActiveUpdatesState::new();
        state.mark_started(ActiveMapUpdate::new(
            "A".to_string(),
            7,
            SourceKind::Workshop,
        ));
        state.set_progress(
            7,
            UpdateProgressPatch {
                phase: Some(UpdatePhase::Downloading),
                bytes_downloaded: Some(50),
                bytes_total: Some(Some(100)),
                detail: Some(Some("file.vpk".to_string())),
            },
        );
        let list = state.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].bytes_downloaded, 50);
        assert_eq!(list[0].bytes_total, Some(100));
        assert_eq!(list[0].percent, Some(50));
        assert_eq!(list[0].detail.as_deref(), Some("file.vpk"));
        assert_eq!(list[0].phase, UpdatePhase::Downloading);

        state.set_progress(
            7,
            UpdateProgressPatch {
                phase: Some(UpdatePhase::Installing),
                bytes_downloaded: None,
                bytes_total: None,
                detail: None,
            },
        );
        assert_eq!(state.list()[0].phase, UpdatePhase::Installing);
        assert_eq!(state.list()[0].percent, Some(50));
    }
}
