//! Filesystem change hints for one library root.
//!
//! Watcher events are *hints*, never facts: the OS can coalesce, drop or
//! reorder them (buffer overflow, network or removable volumes, sleep). The
//! service therefore never applies an event to the index. It only schedules
//! an incremental rescan, which compares the real directory tree against the
//! index, and it also rescans on a timer to repair anything missed.

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;

pub(crate) struct RootWatcher {
    // Dropping the watcher stops delivery.
    _watcher: RecommendedWatcher,
}

impl RootWatcher {
    /// `on_hint(failed)` runs on the notify thread. `failed` reports a watcher
    /// error, which the service treats as a reason to re-check the root.
    pub(crate) fn start(
        root: &Path,
        on_hint: impl Fn(bool) + Send + 'static,
    ) -> notify::Result<Self> {
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                match event {
                    // Reads do not change the library.
                    Ok(event) if matches!(event.kind, EventKind::Access(_)) => {}
                    Ok(_) => on_hint(false),
                    Err(_) => on_hint(true),
                }
            })?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        Ok(Self { _watcher: watcher })
    }
}
