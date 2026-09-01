//! Explorer look for the sync folders.
//!
//! - The sync **root** gets its icon from the sync-root *registration*
//!   (`SyncRootInfo::with_icon`, see `cf.rs`) — that needs an icon file on disk,
//!   which `ensure_icon_file` drops once into the app config dir.
//! - Each **drive subfolder** gets a `desktop.ini` pointing at the same icon
//!   (the registration icon only covers the root). The `.ini` is written after
//!   the placeholder directory exists and flagged hidden+system; `desktop.ini`
//!   is excluded from the upload watcher (see `cf.rs::state_changed`).

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const FOLDER_ICO: &[u8] = include_bytes!("../../icons/drivecord-folder.ico");

/// Name we never upload (see the watcher exclusion in `cf.rs`).
pub const DESKTOP_INI: &str = "desktop.ini";

/// Write the icon to `<config_dir>/drivecord-folder.ico` if needed; return its path.
pub fn ensure_icon_file(config_dir: &Path) -> Option<PathBuf> {
    let dest = config_dir.join("drivecord-folder.ico");
    let up_to_date = fs::metadata(&dest)
        .map(|m| m.len() == FOLDER_ICO.len() as u64)
        .unwrap_or(false);
    if !up_to_date {
        fs::create_dir_all(config_dir).ok()?;
        fs::write(&dest, FOLDER_ICO).ok()?;
    }
    Some(dest)
}

/// Point `dir`'s Explorer icon at `icon_path` via a hidden+system `desktop.ini`.
/// Assumes `dir` already exists. Idempotent — no-ops once applied.
pub fn apply_icon(dir: &Path, icon_path: &Path) {
    let ini = dir.join(DESKTOP_INI);
    let want = format!(
        "[.ShellClassInfo]\r\nIconResource={},0\r\nConfirmFileOp=0\r\n",
        icon_path.display()
    );
    if fs::read_to_string(&ini).ok().as_deref() == Some(want.as_str()) {
        return;
    }
    let _ = Command::new("attrib").args(["-h", "-s"]).arg(&ini).status();
    if fs::write(&ini, want.as_bytes()).is_err() {
        return;
    }
    let _ = Command::new("attrib").args(["+h", "+s"]).arg(&ini).status();
    // Explorer only honours desktop.ini on folders carrying the system bit.
    let _ = Command::new("attrib").arg("+s").arg(dir).status();
}
