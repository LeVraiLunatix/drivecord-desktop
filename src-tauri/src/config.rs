//! Two stores:
//!   - the API key → OS keychain (Windows Credential Manager) via `keyring`
//!   - everything else → `config.json` via `tauri-plugin-store`

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

use crate::error::{AppError, AppResult};

const KEYRING_SERVICE: &str = "drivecord-desktop";
const KEYRING_ACCOUNT: &str = "api-key";
const STORE_FILE: &str = "config.json";
const STORE_KEY: &str = "config";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    /// Bare origin of the Drivecord server, no trailing slash.
    pub server_url: String,
    /// Absolute path of the local folder kept in sync.
    pub sync_dir: String,
    #[serde(default = "default_poll")]
    pub poll_interval_secs: u64,
    #[serde(default)]
    pub excludes: Vec<String>,
}

fn default_poll() -> u64 {
    60
}

// ── Secret ──────────────────────────────────────────────────────────────────

fn entry() -> AppResult<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT).map_err(AppError::from)
}

pub fn save_api_key(key: &str) -> AppResult<()> {
    entry()?.set_password(key).map_err(AppError::from)
}

pub fn load_api_key() -> AppResult<Option<String>> {
    match entry()?.get_password() {
        Ok(k) => Ok(Some(k)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(AppError::from(e)),
    }
}

pub fn delete_api_key() -> AppResult<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(AppError::from(e)),
    }
}

// ── Non-secret ──────────────────────────────────────────────────────────────

pub fn load_config<R: Runtime>(app: &AppHandle<R>) -> AppResult<Option<AppConfig>> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| AppError::Config(e.to_string()))?;
    match store.get(STORE_KEY) {
        Some(v) => serde_json::from_value(v)
            .map(Some)
            .map_err(|e| AppError::Config(e.to_string())),
        None => Ok(None),
    }
}

pub fn save_config<R: Runtime>(app: &AppHandle<R>, cfg: &AppConfig) -> AppResult<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| AppError::Config(e.to_string()))?;
    let value = serde_json::to_value(cfg).map_err(|e| AppError::Config(e.to_string()))?;
    store.set(STORE_KEY, value);
    store.save().map_err(|e| AppError::Config(e.to_string()))?;
    Ok(())
}
