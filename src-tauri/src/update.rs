//! Update detection + auto-download.
//!
//! Checks `tauri.conf.json`'s `plugins.updater.endpoints` (a `latest.json`
//! manifest published alongside each GitHub release, signed with the
//! project's minisign key — see `.github/workflows/release.yml`) on a timer,
//! downloads a newer build silently in the background, then waits for the
//! user to apply it (tray menu entry / in-app banner) rather than restarting
//! unannounced. Mirrors the shape of `sync::SyncEngine`.

use std::{sync::Arc, time::Duration};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

const STATUS_EVENT: &str = "update://status";
/// How often to poll for a new version once the app is running.
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// Give the shell/login flow a moment before the first check.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// "idle" | "checking" | "downloading" | "ready" | "error"
    pub state: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub progress: f32,
    pub error: Option<String>,
}

/// A checked update, paired with its already-downloaded installer bytes.
type PendingUpdate = (tauri_plugin_updater::Update, Vec<u8>);

pub struct UpdateEngine {
    app: AppHandle,
    status: Arc<RwLock<UpdateStatus>>,
    // Holds the checked update + its downloaded bytes until the user (or an
    // automatic policy) triggers `install_now`.
    pending: Arc<tokio::sync::Mutex<Option<PendingUpdate>>>,
}

impl UpdateEngine {
    pub fn init(app: &AppHandle) -> Arc<Self> {
        let engine = Arc::new(Self {
            app: app.clone(),
            status: Arc::new(RwLock::new(UpdateStatus {
                state: "idle".into(),
                ..Default::default()
            })),
            pending: Arc::new(tokio::sync::Mutex::new(None)),
        });

        let loop_engine = engine.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(FIRST_CHECK_DELAY).await;
            loop {
                loop_engine.check_and_download().await;
                tokio::time::sleep(CHECK_INTERVAL).await;
            }
        });

        engine
    }

    pub fn status(&self) -> UpdateStatus {
        self.status.read().clone()
    }

    fn set_status(&self, f: impl FnOnce(&mut UpdateStatus)) {
        {
            let mut s = self.status.write();
            f(&mut s);
        }
        let snapshot = self.status.read().clone();
        let _ = self.app.emit(STATUS_EVENT, snapshot);
    }

    pub async fn check_and_download(&self) {
        self.set_status(|s| s.state = "checking".into());

        let updater = match self.app.updater() {
            Ok(u) => u,
            Err(e) => {
                self.set_status(|s| {
                    s.state = "error".into();
                    s.error = Some(e.to_string());
                });
                return;
            }
        };

        let checked = match updater.check().await {
            Ok(Some(u)) => u,
            Ok(None) => {
                self.set_status(|s| {
                    s.state = "idle".into();
                    s.error = None;
                });
                return;
            }
            Err(e) => {
                self.set_status(|s| {
                    s.state = "error".into();
                    s.error = Some(e.to_string());
                });
                return;
            }
        };

        let version = checked.version.clone();
        let notes = checked.body.clone();
        self.set_status(|s| {
            s.state = "downloading".into();
            s.version = Some(version.clone());
            s.notes = notes.clone();
            s.progress = 0.0;
            s.error = None;
        });

        let mut downloaded: u64 = 0;
        let total_hint = Arc::new(RwLock::new(0u64));
        let status_for_progress = self.status.clone();
        let app_for_progress = self.app.clone();
        let total_hint_p = total_hint.clone();

        let bytes = checked
            .download(
                move |chunk_len, content_len| {
                    downloaded += chunk_len as u64;
                    if let Some(total) = content_len {
                        *total_hint_p.write() = total;
                    }
                    let total = *total_hint_p.read();
                    let frac = if total > 0 {
                        (downloaded as f32 / total as f32).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    status_for_progress.write().progress = frac;
                    let snapshot = status_for_progress.read().clone();
                    let _ = app_for_progress.emit(STATUS_EVENT, snapshot);
                },
                || {},
            )
            .await;

        let bytes = match bytes {
            Ok(b) => b,
            Err(e) => {
                self.set_status(|s| {
                    s.state = "error".into();
                    s.error = Some(e.to_string());
                });
                return;
            }
        };

        *self.pending.lock().await = Some((checked, bytes));
        self.set_status(|s| {
            s.state = "ready".into();
            s.progress = 1.0;
        });

        use tauri_plugin_notification::NotificationExt;
        let _ = self
            .app
            .notification()
            .builder()
            .title("Mise à jour Drivecord disponible")
            .body(format!(
                "La version {version} est téléchargée — redémarre l'app pour l'installer."
            ))
            .show();
    }

    /// Install the already-downloaded update and restart the app. Errors if
    /// nothing is ready yet.
    pub async fn install_now(&self) -> Result<(), String> {
        let pending = self.pending.lock().await.take();
        let Some((update, bytes)) = pending else {
            return Err("Aucune mise à jour prête.".into());
        };
        update.install(&bytes).map_err(|e| e.to_string())?;
        // `AppHandle::restart` returns `!` (it exits the process), which
        // coerces to whatever this function needs to return.
        self.app.restart();
    }
}
