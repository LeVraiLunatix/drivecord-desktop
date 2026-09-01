#[cfg(windows)]
mod sync;
mod token;
mod tray;

use std::sync::Arc;

use tauri::{Manager, Runtime, WebviewWindow};

/// Where the embedded shell (`frontendDist`) is served.
#[cfg(windows)]
const SHELL_URL: &str = "http://tauri.localhost/";
#[cfg(not(windows))]
const SHELL_URL: &str = "tauri://localhost/";

/// First screen when there's no token — branded welcome, then the real
/// first-party login/register flow runs from there inside the webview.
const WELCOME_URL: &str = "https://drivecord.app/desktop-welcome";

fn main_window<R: Runtime>(app: &tauri::AppHandle<R>) -> Option<WebviewWindow<R>> {
    app.get_webview_window("main")
}

fn navigate<R: Runtime>(app: &tauri::AppHandle<R>, url: &str) {
    if let Some(win) = main_window(app) {
        if let Ok(parsed) = url.parse() {
            let _ = win.navigate(parsed);
        }
    }
}

/// Called by the remote login page once it holds a fresh token (already saved
/// via `save_token`): switch the window to the embedded shell.
#[tauri::command]
fn enter_shell<R: Runtime>(app: tauri::AppHandle<R>) {
    navigate(&app, SHELL_URL);
}

/// Send the window (back) to the first-party login page.
#[tauri::command]
fn enter_auth<R: Runtime>(app: tauri::AppHandle<R>) {
    navigate(&app, WELCOME_URL);
}

/// Clear the stored token and return to login.
#[tauri::command]
fn logout<R: Runtime>(app: tauri::AppHandle<R>) -> Result<(), String> {
    token::clear_token()?;
    navigate(&app, WELCOME_URL);
    Ok(())
}

// ── Custom window controls (the OS title bar is hidden — `decorations: false`) ──

#[tauri::command]
fn win_minimize<R: Runtime>(app: tauri::AppHandle<R>) {
    if let Some(w) = main_window(&app) {
        let _ = w.minimize();
    }
}

#[tauri::command]
fn win_toggle_maximize<R: Runtime>(app: tauri::AppHandle<R>) {
    if let Some(w) = main_window(&app) {
        if w.is_maximized().unwrap_or(false) {
            let _ = w.unmaximize();
        } else {
            let _ = w.maximize();
        }
    }
}

#[tauri::command]
fn win_close<R: Runtime>(app: tauri::AppHandle<R>) {
    if let Some(w) = main_window(&app) {
        let _ = w.close();
    }
}

#[tauri::command]
fn win_start_drag<R: Runtime>(app: tauri::AppHandle<R>) {
    if let Some(w) = main_window(&app) {
        let _ = w.start_dragging();
    }
}

fn focus_main<R: Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(w) = main_window(app) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

// ── Folder-sync engine (kDrive-style — Windows only) ─────────────────────────

#[tauri::command]
fn sync_status(state: tauri::State<'_, Arc<sync::SyncEngine>>) -> sync::SyncStatus {
    state.status()
}

#[tauri::command]
fn sync_pick_folder(app: tauri::AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let path = app.dialog().file().blocking_pick_folder()?;
    path.into_path().ok().map(|p| p.display().to_string())
}

#[tauri::command]
fn sync_set_root(state: tauri::State<'_, Arc<sync::SyncEngine>>, path: String) {
    state.set_root(std::path::PathBuf::from(path));
}

#[tauri::command]
fn sync_enable(state: tauri::State<'_, Arc<sync::SyncEngine>>) {
    state.start();
}

#[tauri::command]
fn sync_disable(state: tauri::State<'_, Arc<sync::SyncEngine>>) {
    state.stop();
}

#[tauri::command]
fn sync_now(state: tauri::State<'_, Arc<sync::SyncEngine>>) {
    state.reconcile_now();
}

#[tauri::command]
fn sync_set_drive_enabled(state: tauri::State<'_, Arc<sync::SyncEngine>>, drive_id: String, enabled: bool) {
    state.set_drive_enabled(drive_id, enabled);
}

#[tauri::command]
fn sync_open_folder(state: tauri::State<'_, Arc<sync::SyncEngine>>) -> Result<(), String> {
    let root = state.status().root.ok_or_else(|| "Aucun dossier choisi.".to_string())?;
    std::process::Command::new("explorer")
        .arg(&root)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
                focus_main(app);
            }))
            .plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                Some(vec!["--minimized"]),
            ));
    }

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            #[cfg(windows)]
            {
                let engine = sync::SyncEngine::init(app.handle());
                app.manage(engine);
            }

            tray::create_tray(app.handle())?;

            // No token yet → send the window to first-party login. With a token,
            // the window keeps its default content (the embedded shell).
            if token::read().is_none() {
                navigate(app.handle(), WELCOME_URL);
            }
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            token::save_token,
            token::get_token,
            token::clear_token,
            enter_shell,
            enter_auth,
            logout,
            win_minimize,
            win_toggle_maximize,
            win_close,
            win_start_drag,
                        sync_status,
                        sync_pick_folder,
                        sync_set_root,
                        sync_enable,
                        sync_disable,
                        sync_now,
                        sync_set_drive_enabled,
                        sync_open_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
