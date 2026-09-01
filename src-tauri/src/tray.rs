use std::sync::Arc;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Listener, Manager, Runtime,
};

use crate::sync::{SyncEngine, SyncStatus};

fn state_label(status: &SyncStatus) -> String {
    if !status.enabled {
        return "État : en pause".into();
    }
    match status.state.as_str() {
        "syncing" => "État : synchronisation…".into(),
        "synced" => "État : synchronisé".into(),
        "error" => format!(
            "État : erreur{}",
            status
                .last_error
                .as_deref()
                .map(|e| format!(" ({e})"))
                .unwrap_or_default()
        ),
        _ => "État : inactif".into(),
    }
}

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

    // Keep the sync engine's status reflected live in the tray menu.
    {
        let pause = pause.clone();
        let status_item = status.clone();
        app.listen("sync://status", move |event| {
            let Ok(s) = serde_json::from_str::<SyncStatus>(event.payload()) else { return };
            let _ = status_item.set_text(state_label(&s));
            let _ = pause.set_text(if s.enabled {
                "Mettre la synchro en pause"
            } else {
                "Reprendre la synchro"
            });
        });
    }

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
            "pause" => {
                if let Some(engine) = app.try_state::<Arc<SyncEngine>>() {
                    if engine.status().enabled {
                        engine.stop();
                    } else {
                        engine.start();
                    }
                }
            }
            "folder" => {
                if let Some(engine) = app.try_state::<Arc<SyncEngine>>() {
                    if let Some(root) = engine.status().root {
                        let _ = std::process::Command::new("explorer").arg(root).spawn();
                    }
                }
            }
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
