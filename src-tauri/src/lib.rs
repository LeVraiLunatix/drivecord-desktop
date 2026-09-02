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

// ── Custom window controls (OS title bar hidden — `decorations: false`) ──
// Act on the *calling* window (Tauri injects it), so the same traffic lights
// drive the main shell and the standalone "Import Drivecord" window.

#[tauri::command]
fn win_minimize<R: Runtime>(window: WebviewWindow<R>) {
    let _ = window.minimize();
}

#[tauri::command]
fn win_toggle_maximize<R: Runtime>(window: WebviewWindow<R>) {
    if window.is_maximized().unwrap_or(false) {
        let _ = window.unmaximize();
    } else {
        let _ = window.maximize();
    }
}

#[tauri::command]
fn win_close<R: Runtime>(window: WebviewWindow<R>) {
    let _ = window.close();
}

#[tauri::command]
fn win_start_drag<R: Runtime>(window: WebviewWindow<R>) {
    let _ = window.start_dragging();
}

/// Minimise the app to the tray (folder sync keeps running).
#[tauri::command]
fn app_hide<R: Runtime>(app: tauri::AppHandle<R>) {
    if let Some(w) = main_window(&app) {
        let _ = w.hide();
    }
}

/// Quit the whole app.
#[tauri::command]
fn app_quit<R: Runtime>(app: tauri::AppHandle<R>) {
    app.exit(0);
}

fn focus_main<R: Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(w) = main_window(app) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

// ── Folder-sync engine (kDrive-style — Windows only) ─────────────────────────

#[tauri::command(async)]
fn sync_status(state: tauri::State<'_, Arc<sync::SyncEngine>>) -> sync::SyncStatus {
    state.status()
}

#[tauri::command]
async fn sync_pick_folder(app: tauri::AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    // Callback form (not `blocking_pick_folder`): the plugin marshals the
    // native folder dialog onto the right thread itself. Calling the blocking
    // variant from a command's worker thread trips COM apartment issues on
    // Windows.
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |picked| {
        let _ = tx.send(picked);
    });
    rx.await
        .ok()
        .flatten()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string())
}

#[tauri::command(async)]
fn sync_set_root(state: tauri::State<'_, Arc<sync::SyncEngine>>, path: String) {
    state.set_root(std::path::PathBuf::from(path));
}

#[tauri::command(async)]
fn sync_enable(state: tauri::State<'_, Arc<sync::SyncEngine>>) {
    state.start();
}

#[tauri::command(async)]
fn sync_disable(state: tauri::State<'_, Arc<sync::SyncEngine>>) {
    state.stop();
}

#[tauri::command(async)]
fn sync_now(state: tauri::State<'_, Arc<sync::SyncEngine>>) {
    state.reconcile_now();
}

#[tauri::command(async)]
fn sync_set_drive_enabled(state: tauri::State<'_, Arc<sync::SyncEngine>>, drive_id: String, enabled: bool) {
    state.set_drive_enabled(drive_id, enabled);
}

#[tauri::command(async)]
fn sync_open_folder(state: tauri::State<'_, Arc<sync::SyncEngine>>) -> Result<(), String> {
    let root = state.status().root.ok_or_else(|| "Aucun dossier choisi.".to_string())?;
    std::process::Command::new("explorer")
        .arg(&root)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command(async)]
fn sync_uploads_status(state: tauri::State<'_, Arc<sync::SyncEngine>>) -> Vec<sync::UploadItem> {
    state.uploads_status()
}

#[tauri::command(async)]
fn sync_open_file(
    state: tauri::State<'_, Arc<sync::SyncEngine>>,
    drive_id: String,
    file_id: String,
) -> Result<(), String> {
    let path = state.file_local_path(&drive_id, &file_id).ok_or_else(|| {
        "Fichier pas encore dans le dossier synchronisé — active la synchro et attends qu'elle se termine.".to_string()
    })?;
    // `explorer <file>` launches it with its default app on Windows 10/11.
    std::process::Command::new("explorer")
        .arg(&path)
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

            // Closing the main window pops an in-app modal ("Réduire" keeps the
            // folder sync running from the tray). We just veto the close and let
            // the shell decide via a DOM event → `app_hide` / `app_quit`.
            if let Some(main) = app.get_webview_window("main") {
                let handle = app.handle().clone();
                main.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        if let Some(w) = handle.get_webview_window("main") {
                            let _ = w.eval(
                                "window.dispatchEvent(new CustomEvent('drivecord:close-request'))",
                            );
                        }
                    }
                });
            }

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
            app_hide,
            app_quit,
                        sync_status,
                        sync_pick_folder,
                        sync_set_root,
                        sync_enable,
                        sync_disable,
                        sync_now,
                        sync_set_drive_enabled,
                        sync_open_folder,
                        sync_open_file,
                        sync_uploads_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
