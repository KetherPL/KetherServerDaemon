// SPDX-License-Identifier: GPL-3.0-only
pub mod traits;
pub mod inotify;

pub use traits::{Watcher, WatcherEvent};
pub use inotify::InotifyWatcher;

