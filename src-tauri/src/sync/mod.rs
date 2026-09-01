//! kDrive-style folder sync (Phase A + B).
//!
//! One registered Cloud Files sync root = the user's chosen folder. Every
//! drive becomes a real subdirectory under it; every file becomes a
//! placeholder (0 bytes on disk) that hydrates — downloads + AES-GCM
//! decrypts — the moment Explorer opens it. Dropping a real file into a
//! drive's subfolder is detected and uploaded (encrypted) automatically.
//!
//! Everything Cloud-Filter-API-shaped lives on one dedicated OS thread (the
//! `Connection` the crate hands back isn't meant to hop threads) and talks to
//! the rest of the app through plain, `Send`-safe channels + a shared status
//! snapshot. See `cf.rs` for the callback implementation + worker thread,
//! `mirror.rs` for the cloud->disk reconciliation, `upload.rs` for the
//! disk->cloud direction.

mod api;
mod cf;
mod config;
mod crypto;
mod discord;
mod folder_style;
mod mirror;
mod upload;
mod watcher;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

pub use config::SyncConfig;

pub const PROVIDER_NAME: &str = "DrivecordSync";
pub const DISPLAY_NAME: &str = "Drivecord";
const STATUS_EVENT: &str = "sync://status";
const RECONCILE_INTERVAL_SECS: u64 = 60;

/// A drive's secrets, resolved once per reconcile pass from `GET /api/webhooks`.
#[derive(Debug, Clone)]
#[allow(dead_code)] // `name` kept for parity/debugging; the sanitized dir name is what mirror.rs actually uses
pub struct DriveSecrets {
    pub name: String,
    pub webhook_url: String,
    pub enc_key: String,
}

/// In-memory index rebuilt on every reconcile — lets the Cloud Filter
/// callbacks (running on their own thread, no network round-trip budget to
/// spare) answer "what is this placeholder" without hitting the API.
#[derive(Debug, Default)]
pub struct FileIndex {
    pub drives: HashMap<String, DriveSecrets>,
    /// fileId -> (driveId, entry)
    pub files: HashMap<String, (String, api::FileEntry)>,
    /// absolute local path -> (driveId, fileId) — for delete/rename lookups.
    pub path_to_file: HashMap<PathBuf, (String, String)>,
    /// absolute local directory -> (driveId, folderId; "" = drive root).
    pub path_to_folder: HashMap<PathBuf, (String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveStatus {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub file_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub enabled: bool,
    pub running: bool,
    pub root: Option<String>,
    pub drives: Vec<DriveStatus>,
    pub state: String, // "idle" | "syncing" | "synced" | "error"
    pub last_error: Option<String>,
    pub last_sync_at: Option<i64>,
}

pub(crate) enum EngineCmd {
    Stop,
    ReconcileNow,
}

/// Shared context threaded through the worker: plain data + channels only, so
/// it's fine to build on one thread and read from another. Cheap to clone —
/// every field is either `Copy`-ish small data or an `Arc`/`Client` handle.
#[derive(Clone)]
pub(crate) struct EngineCtx {
    pub app: AppHandle,
    pub http: reqwest::Client,
    pub token: String,
    pub root: PathBuf,
    pub config_path: PathBuf,
    pub index: Arc<RwLock<FileIndex>>,
    pub status: Arc<RwLock<SyncStatus>>,
    /// Local paths currently mid-upload — guards against re-triggering on
    /// overlapping `state_changed` events for the same file.
    pub uploading: Arc<RwLock<HashSet<PathBuf>>>,
}

impl EngineCtx {
    pub fn config(&self) -> SyncConfig {
        SyncConfig::load(&self.config_path)
    }

    pub fn touch_status(&self) {
        let snapshot = self.status.read().clone();
        let _ = self.app.emit(STATUS_EVENT, snapshot);
    }

    pub fn set_error(&self, msg: impl Into<String>) {
        let mut s = self.status.write();
        s.state = "error".into();
        s.last_error = Some(msg.into());
        drop(s);
        self.touch_status();
    }
}

pub struct SyncEngine {
    app: AppHandle,
    config_path: PathBuf,
    config: RwLock<SyncConfig>,
    status: Arc<RwLock<SyncStatus>>,
    /// Shared with the worker's `EngineCtx` so the UI can resolve a file's
    /// on-disk path (e.g. "open this heavy video in the synced folder").
    index: Arc<RwLock<FileIndex>>,
    cmd_tx: Arc<RwLock<Option<tokio::sync::mpsc::Sender<EngineCmd>>>>,
    worker: RwLock<Option<std::thread::JoinHandle<()>>>,
}

impl SyncEngine {
    pub fn init(app: &AppHandle) -> Arc<Self> {
        let config_path = app
            .path()
            .app_config_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("sync.json");
        let config = SyncConfig::load(&config_path);
        let engine = Arc::new(Self {
            app: app.clone(),
            config_path,
            status: Arc::new(RwLock::new(SyncStatus {
                enabled: config.enabled,
                root: config.root.as_ref().map(|p| p.display().to_string()),
                state: "idle".into(),
                ..Default::default()
            })),
            config: RwLock::new(config),
            index: Arc::new(RwLock::new(FileIndex::default())),
            cmd_tx: Arc::new(RwLock::new(None)),
            worker: RwLock::new(None),
        });
        if engine.config.read().enabled {
            engine.start();
        }
        engine
    }

    pub fn status(&self) -> SyncStatus {
        self.status.read().clone()
    }

    /// Local path of a synced file, if the mirror has placed it yet.
    pub fn file_local_path(&self, drive_id: &str, file_id: &str) -> Option<PathBuf> {
        let idx = self.index.read();
        idx.path_to_file
            .iter()
            .find(|(_, (d, f))| d == drive_id && f == file_id)
            .map(|(p, _)| p.clone())
    }

    fn save_config(&self, cfg: &SyncConfig) {
        if let Err(e) = cfg.save(&self.config_path) {
            eprintln!("sync: échec de sauvegarde de la config: {e}");
        }
    }

    pub fn set_root(&self, root: PathBuf) {
        let was_running = self.is_running();
        if was_running {
            // Cleanly tear the current sync root down before repointing.
            self.stop_and_join();
        }
        {
            let mut cfg = self.config.write();
            cfg.root = Some(root.clone());
            self.save_config(&cfg);
        }
        self.status.write().root = Some(root.display().to_string());
        if was_running {
            self.start(); // re-registers on the new folder
        } else {
            self.touch();
        }
    }

    pub fn is_running(&self) -> bool {
        self.cmd_tx.read().is_some()
    }

    /// Start the worker thread. No-op if already running or no root chosen.
    pub fn start(&self) {
        if self.is_running() {
            return;
        }
        let Some(root) = self.config.read().root.clone() else {
            self.set_error("Aucun dossier choisi.");
            return;
        };
        if let Err(e) = std::fs::create_dir_all(&root) {
            self.set_error(format!("Impossible de créer le dossier : {e}"));
            return;
        }
        let Some(token) = crate::token::read() else {
            self.set_error("Non connecté.");
            return;
        };

        {
            let mut cfg = self.config.write();
            cfg.enabled = true;
            self.save_config(&cfg);
        }

        let (tx, rx) = tokio::sync::mpsc::channel(32);
        *self.cmd_tx.write() = Some(tx);

        {
            let mut s = self.status.write();
            s.enabled = true;
            s.running = true;
            s.state = "syncing".into();
            s.last_error = None;
        }
        self.touch();

        let ctx = EngineCtx {
            app: self.app.clone(),
            http: reqwest::Client::builder()
                .user_agent("drivecord-desktop-sync")
                // Wide connection pool — hydration fans many parallel chunk
                // GETs at cdn.discordapp.com.
                .pool_max_idle_per_host(32)
                .pool_idle_timeout(std::time::Duration::from_secs(90))
                .build()
                .unwrap_or_default(),
            token,
            root,
            config_path: self.config_path.clone(),
            index: {
                *self.index.write() = FileIndex::default();
                self.index.clone()
            },
            status: self.status.clone(),
            uploading: Arc::new(RwLock::new(HashSet::new())),
        };
        let cmd_tx_slot = self.cmd_tx.clone();
        let app_for_end = self.app.clone();
        let status_for_end = self.status.clone();

        let handle = std::thread::Builder::new()
            .name("drivecord-sync".into())
            .spawn(move || {
                cf::run_worker(ctx, rx);
                *cmd_tx_slot.write() = None;
                let mut s = status_for_end.write();
                s.running = false;
                if s.state != "error" {
                    s.state = "idle".into();
                }
                drop(s);
                let snapshot = status_for_end.read().clone();
                let _ = app_for_end.emit(STATUS_EVENT, snapshot);
            })
            .expect("spawn du thread de synchro");
        *self.worker.write() = Some(handle);
    }

    /// Signal the worker to stop and block until it has fully torn down the
    /// sync root (deregistered) — so the caller can immediately re-`start()`
    /// on a different folder without racing the registration.
    fn stop_and_join(&self) {
        if let Some(tx) = self.cmd_tx.read().clone() {
            let _ = tx.try_send(EngineCmd::Stop);
        }
        let handle = self.worker.write().take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
        *self.cmd_tx.write() = None;
    }

    pub fn stop(&self) {
        {
            let mut cfg = self.config.write();
            cfg.enabled = false;
            self.save_config(&cfg);
        }
        self.stop_and_join();
        let mut s = self.status.write();
        s.enabled = false;
        s.running = false;
        s.state = "idle".into();
        drop(s);
        self.touch();
    }

    pub fn reconcile_now(&self) {
        if let Some(tx) = self.cmd_tx.read().clone() {
            let _ = tx.try_send(EngineCmd::ReconcileNow);
        }
    }

    pub fn set_drive_enabled(&self, drive_id: String, enabled: bool) {
        {
            let mut cfg = self.config.write();
            cfg.drives.insert(drive_id, enabled);
            self.save_config(&cfg);
        }
        self.reconcile_now();
    }

    fn set_error(&self, msg: impl Into<String>) {
        let mut s = self.status.write();
        s.state = "error".into();
        s.last_error = Some(msg.into());
        drop(s);
        self.touch();
    }

    fn touch(&self) {
        let snapshot = self.status.read().clone();
        let _ = self.app.emit(STATUS_EVENT, snapshot);
    }
}
