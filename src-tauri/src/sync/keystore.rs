//! Drive keys handed over by the unlocked shell (`sync_set_drive_keys`), kept so
//! the sync engine also works when the app starts minimized at login, before
//! anyone unlocks the web UI.
//!
//! On disk: `<app config>/drive-keys.bin` = `seal(localKey, JSON {driveId: b64 DK})`.
//! `localKey` (32 random bytes) lives in the Windows Credential Manager next to
//! the session token — same protection level — and is deleted on sign-out,
//! which makes the file unreadable even if deleting it failed.

use std::{collections::HashMap, path::Path};

use base64::{engine::general_purpose::STANDARD as B64, Engine};

use super::e2ee::{self, RawKey};

const SERVICE: &str = "drivecord-desktop";
const ACCOUNT: &str = "drive-keys-key";
const CONTEXT: &str = "drivecord-desktop:drive-keys:v1";
pub const FILE_NAME: &str = "drive-keys.bin";

fn entry() -> Option<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT).ok()
}

fn local_key(create: bool) -> Option<RawKey> {
    let entry = entry()?;
    if let Ok(k) = entry.get_password() {
        if let Ok(k) = e2ee::key_from_b64(&k) {
            return Some(k);
        }
    }
    if !create {
        return None;
    }
    let k = e2ee::random_key();
    entry.set_password(&B64.encode(k)).ok()?;
    Some(k)
}

pub fn load(dir: &Path) -> HashMap<String, RawKey> {
    let Some(key) = local_key(false) else {
        return HashMap::new();
    };
    let Ok(blob) = std::fs::read_to_string(dir.join(FILE_NAME)) else {
        return HashMap::new();
    };
    let Ok(json) = e2ee::open(&key, blob.trim(), CONTEXT) else {
        return HashMap::new();
    };
    let map: HashMap<String, String> = serde_json::from_slice(&json).unwrap_or_default();
    map.into_iter()
        .filter_map(|(id, k)| e2ee::key_from_b64(&k).ok().map(|k| (id, k)))
        .collect()
}

/// Merge `keys` into the stored set. Returns true if anything changed.
pub fn merge(dir: &Path, keys: HashMap<String, RawKey>) -> Result<bool, String> {
    let mut all = load(dir);
    let before = all.len();
    let mut changed = false;
    for (id, k) in keys {
        if all.insert(id, k) != Some(k) {
            changed = true;
        }
    }
    if !changed && all.len() == before {
        return Ok(false);
    }
    let key = local_key(true).ok_or("trousseau Windows indisponible")?;
    let json: HashMap<&String, String> = all.iter().map(|(id, k)| (id, B64.encode(k))).collect();
    let blob = e2ee::seal(
        &key,
        &serde_json::to_vec(&json).map_err(|e| e.to_string())?,
        CONTEXT,
    )?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(FILE_NAME), blob).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Forget every stored drive key (sign-out).
pub fn clear(dir: Option<&Path>) {
    if let Some(entry) = entry() {
        let _ = entry.delete_credential();
    }
    if let Some(dir) = dir {
        let _ = std::fs::remove_file(dir.join(FILE_NAME));
    }
}
