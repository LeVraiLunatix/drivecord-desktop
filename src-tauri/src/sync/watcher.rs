//! Filesystem watcher for the disk -> cloud direction.
//!
//! The `cloud-filter` crate's built-in watcher only reports attribute changes
//! (pin / unpin), so it never sees a file the user drops into a drive folder.
//! A plain recursive `notify` watch on the sync root fills that gap: any
//! create / write / rename under the root is handed to
//! `cf::consider_local_path`, which figures out if it's genuinely new content
//! and uploads it.

use std::path::PathBuf;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// Start watching `root` recursively. The returned watcher must be kept alive
/// for the watch to stay active. Paths land on `tx`.
pub fn spawn(
    root: &std::path::Path,
    tx: tokio::sync::mpsc::UnboundedSender<PathBuf>,
) -> notify::Result<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        use notify::EventKind::*;
        if !matches!(event.kind, Create(_) | Modify(_)) {
            return;
        }
        for p in event.paths {
            let _ = tx.send(p);
        }
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    Ok(watcher)
}
