// SPDX-License-Identifier: GPL-3.0-only
pub mod traits;
pub mod inotify;
pub mod debounce;

pub use traits::{Watcher, WatcherEvent};
pub use inotify::InotifyWatcher;
pub use debounce::{PendingEntry, schedule_pending, should_force_sync};

