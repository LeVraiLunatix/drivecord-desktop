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

/// Bound on the number of paths queued between the `notify` callback thread
/// and the consumer in `cf.rs`. Unbounded would let a huge drag-and-drop (or
/// a slow consumer) grow this queue without limit; this is generous enough
/// that a normal burst never hits it, while still capping worst-case memory.
const CHANNEL_CAPACITY: usize = 10_000;

/// Start watching `root` recursively. The returned watcher must be kept alive
/// for the watch to stay active. Paths land on `tx`.
pub fn spawn(
    root: &std::path::Path,
    tx: tokio::sync::mpsc::Sender<PathBuf>,
) -> notify::Result<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        use notify::EventKind::*;
        if !matches!(event.kind, Create(_) | Modify(_)) {
            return;
        }
        for p in event.paths {
            // This callback runs synchronously on notify's own dedicated
            // thread (not a tokio task), so blocking here just applies
            // backpressure to the filesystem events, not to the async
            // runtime — exactly what we want when the consumer falls behind
            // instead of buffering unboundedly.
            if let Err(e) = tx.blocking_send(p) {
                eprintln!("sync: watcher fs — file d'événements fermée, événement perdu : {e}");
            }
        }
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    Ok(watcher)
}

/// Convenience for callers that just need a channel sized per `CHANNEL_CAPACITY`.
pub fn channel() -> (tokio::sync::mpsc::Sender<PathBuf>, tokio::sync::mpsc::Receiver<PathBuf>) {
    tokio::sync::mpsc::channel(CHANNEL_CAPACITY)
}
