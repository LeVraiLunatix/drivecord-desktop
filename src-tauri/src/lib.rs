mod token;
mod tray;

use tauri::{Manager, Runtime, WebviewWindow};

/// Where the embedded shell (`frontendDist`) is served.
#[cfg(windows)]
const SHELL_URL: &str = "http://tauri.localhost/";
#[cfg(not(windows))]
const SHELL_URL: &str = "tauri://localhost/";

/// First-party login page (real NextAuth flow runs here, in the webview).
const LOGIN_URL: &str = "https://drivecord.app/login?callbackUrl=%2Fdrive";

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
    navigate(&app, LOGIN_URL);
}

/// Clear the stored token and return to login.
#[tauri::command]
fn logout<R: Runtime>(app: tauri::AppHandle<R>) -> Result<(), String> {
    token::clear_token()?;
    navigate(&app, LOGIN_URL);
    Ok(())
}

fn focus_main<R: Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(w) = main_window(app) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
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
            tray::create_tray(app.handle())?;

            // No token yet → send the window to first-party login. With a token,
            // the window keeps its default content (the embedded shell).
            if token::read().is_none() {
                navigate(app.handle(), LOGIN_URL);
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
