use std::sync::Arc;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
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

/// Rebuild the whole tray menu from the current sync status (drive list can
/// change, so a static menu won't do).
fn build_menu<R: Runtime>(app: &AppHandle<R>, status: &SyncStatus) -> tauri::Result<Menu<R>> {
    let open = MenuItem::with_id(app, "open", "Ouvrir Drivecord", true, None::<&str>)?;
    let folder = MenuItem::with_id(app, "folder", "Ouvrir le dossier dans Windows", true, None::<&str>)?;
    let pause = MenuItem::with_id(
        app,
        "pause",
        if status.enabled {
            "Mettre la synchro en pause"
        } else {
            "Reprendre la synchro"
        },
        true,
        None::<&str>,
    )?;
    let state = MenuItem::with_id(app, "status", state_label(status), false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quitter Drivecord", true, None::<&str>)?;

    let menu = Menu::new(app)?;
    menu.append(&open)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    if status.drives.is_empty() {
        let none = MenuItem::with_id(app, "drives_none", "Aucun drive synchronisé", false, None::<&str>)?;
        menu.append(&none)?;
    } else {
        let header = MenuItem::with_id(app, "drives_header", "Ouvrir un drive", false, None::<&str>)?;
        menu.append(&header)?;
        for d in &status.drives {
            let it = MenuItem::with_id(
                app,
                format!("drive:{}", d.id),
                format!("   {}", d.name),
                true,
                None::<&str>,
            )?;
            menu.append(&it)?;
        }
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&folder)?;
    menu.append(&pause)?;
    menu.append(&state)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&quit)?;
    Ok(menu)
}

pub fn create_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let status0 = app
        .try_state::<Arc<SyncEngine>>()
        .map(|e| e.status())
        .unwrap_or_default();
    let menu = build_menu(app, &status0)?;

    let tray = TrayIconBuilder::with_id("main")
        .tooltip("Drivecord")
        .icon(
            app.default_window_icon()
                .cloned()
                .expect("un icône de fenêtre par défaut est requis"),
        )
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id.as_ref();
            match id {
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
                "folder" => open_sync_folder(app),
                _ if id.starts_with("drive:") => open_drive(app, &id["drive:".len()..]),
                _ => {}
            }
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

    // Rebuild the menu whenever sync status changes (drive list, pause state…).
    let app_l = app.clone();
    let tray_l: TrayIcon<R> = tray.clone();
    app.listen("sync://status", move |event| {
        let Ok(s) = serde_json::from_str::<SyncStatus>(event.payload()) else { return };
        if let Ok(menu) = build_menu(&app_l, &s) {
            let _ = tray_l.set_menu(Some(menu));
        }
    });

    Ok(())
}

fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn open_sync_folder<R: Runtime>(app: &AppHandle<R>) {
    if let Some(engine) = app.try_state::<Arc<SyncEngine>>() {
        if let Some(root) = engine.status().root {
            let _ = std::process::Command::new("explorer").arg(root).spawn();
        }
    }
}

/// Bring the shell forward and navigate it to `/drive?open=<id>`; the drive page
/// reads `open` on mount and selects that drive.
fn open_drive<R: Runtime>(app: &AppHandle<R>, drive_id: &str) {
    if let Some(w) = app.get_webview_window("main") {
        if let Ok(mut url) = w.url() {
            url.set_path("/drive");
            url.set_query(Some(&format!("open={drive_id}")));
            let _ = w.navigate(url);
        }
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
