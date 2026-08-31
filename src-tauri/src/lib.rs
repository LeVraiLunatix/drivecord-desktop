// The `api` client surface (uploads, download, public links, wire models) is
// intentionally complete ahead of its callers — commands wire the rest in
// B5–B8. Drop this once they're all reachable.
#[allow(dead_code)]
mod api;
mod commands;
mod config;
mod error;
mod tray;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        // single-instance must be the first plugin registered.
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
        .plugin(tauri_plugin_store::Builder::default().build())
        .setup(|app| {
            tray::create_tray(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::verify_key,
            commands::set_api_key,
            commands::has_api_key,
            commands::clear_api_key,
            commands::get_config,
            commands::set_config,
            commands::pick_sync_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn focus_main<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
