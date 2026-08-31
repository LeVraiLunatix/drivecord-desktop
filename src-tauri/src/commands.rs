//! Commands exposed to the frontend via `invoke`.

use tauri::{AppHandle, Runtime};
use tauri_plugin_dialog::DialogExt;

use crate::api::{models::MeResponse, ApiClient};
use crate::config::{self, AppConfig};
use crate::error::{AppError, AppResult};

/// Check a server URL + API key against `GET /api/v1/me`. Persists nothing.
#[tauri::command]
pub async fn verify_key(server_url: String, api_key: String) -> AppResult<MeResponse> {
    ApiClient::new(&server_url, &api_key)?.me().await
}

#[tauri::command]
pub fn set_api_key(api_key: String) -> AppResult<()> {
    if !api_key.starts_with("dvc_") {
        return Err(AppError::Config(
            "format de clé inattendu (préfixe « dvc_ » attendu)".into(),
        ));
    }
    config::save_api_key(&api_key)
}

#[tauri::command]
pub fn has_api_key() -> AppResult<bool> {
    Ok(config::load_api_key()?.is_some())
}

#[tauri::command]
pub fn clear_api_key() -> AppResult<()> {
    config::delete_api_key()
}

#[tauri::command]
pub fn get_config<R: Runtime>(app: AppHandle<R>) -> AppResult<Option<AppConfig>> {
    config::load_config(&app)
}

#[tauri::command]
pub fn set_config<R: Runtime>(app: AppHandle<R>, config: AppConfig) -> AppResult<()> {
    config::save_config(&app, &config)
}

/// Native folder picker. `None` if the user cancels.
#[tauri::command]
pub async fn pick_sync_dir<R: Runtime>(app: AppHandle<R>) -> AppResult<Option<String>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |picked| {
        let _ = tx.send(picked);
    });
    let picked = rx.await.map_err(|e| AppError::Other(e.to_string()))?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned()))
}
