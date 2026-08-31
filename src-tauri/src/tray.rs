use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};
use tauri_plugin_opener::OpenerExt;

use crate::config;

pub fn create_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Ouvrir Drivecord", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Mettre la synchro en pause", true, None::<&str>)?;
    let status = MenuItem::with_id(app, "status", "État : inactif", false, None::<&str>)?;
    let folder = MenuItem::with_id(app, "folder", "Ouvrir le dossier local", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
    let sep_a = PredefinedMenuItem::separator(app)?;
    let sep_b = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[&open, &pause, &status, &sep_a, &folder, &sep_b, &quit],
    )?;

    let _tray = TrayIconBuilder::with_id("main")
        .tooltip("Drivecord")
        .icon(
            app.default_window_icon()
                .cloned()
                .expect("un icône de fenêtre par défaut est requis"),
        )
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "quit" => app.exit(0),
            "folder" => {
                if let Ok(Some(cfg)) = config::load_config(app) {
                    let _ = app.opener().open_path(cfg.sync_dir, None::<&str>);
                }
            }
            // "pause" is wired once the sync engine exists (B5+).
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
