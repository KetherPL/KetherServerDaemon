// SPDX-License-Identifier: GPL-3.0-only
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tracing::warn;

pub const MAX_PENDING: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingEntry {
    pub deadline: Instant,
    pub first_seen: Instant,
}

impl PendingEntry {
    pub fn new(now: Instant, debounce_window: Duration) -> Self {
        Self {
            deadline: now + debounce_window,
            first_seen: now,
        }
    }

    pub fn refresh_deadline(&mut self, now: Instant, debounce_window: Duration) {
        self.deadline = now + debounce_window;
    }
}

/// Insert or refresh a debounced path, evicting the oldest entry when at capacity.
pub fn schedule_pending(
    pending: &mut HashMap<PathBuf, PendingEntry>,
    last_unstable_log: &mut HashMap<PathBuf, Instant>,
    path: PathBuf,
    now: Instant,
    debounce_window: Duration,
) {
    if pending.len() >= MAX_PENDING && !pending.contains_key(&path) {
        if let Some(oldest_path) = pending
            .iter()
            .min_by_key(|(_, entry)| entry.first_seen)
            .map(|(p, _)| p.clone())
        {
            warn!(
                path = %oldest_path.display(),
                cap = MAX_PENDING,
                "Pending watcher queue full, evicting oldest path"
            );
            pending.remove(&oldest_path);
            last_unstable_log.remove(&oldest_path);
        }
    }

    pending
        .entry(path)
        .and_modify(|entry| entry.refresh_deadline(now, debounce_window))
        .or_insert_with(|| PendingEntry::new(now, debounce_window));
}

/// Returns true when a path has waited longer than `max_stable_wait` for stability.
pub fn should_force_sync(first_seen: Instant, now: Instant, max_stable_wait: Duration) -> bool {
    now.duration_since(first_seen) > max_stable_wait
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_force_sync_false_within_window() {
        let start = Instant::now();
        let now = start + Duration::from_secs(30);
        assert!(!should_force_sync(start, now, Duration::from_secs(60)));
    }

    #[test]
    fn test_should_force_sync_true_after_window() {
        let start = Instant::now();
        let now = start + Duration::from_secs(61);
        assert!(should_force_sync(start, now, Duration::from_secs(60)));
    }
}
