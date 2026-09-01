//! Persisted sync settings — `%APPDATA%/app.drivecord.desktop/sync.json`.

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncConfig {
    pub enabled: bool,
    pub root: Option<PathBuf>,
    /// driveId -> opted in. Missing entry defaults to "included".
    #[serde(default)]
    pub drives: HashMap<String, bool>,
}

impl SyncConfig {
    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default())?;
        std::fs::rename(&tmp, path)
    }

    pub fn drive_enabled(&self, drive_id: &str) -> bool {
        *self.drives.get(drive_id).unwrap_or(&true)
    }
}
