//! The bearer session token (a NextAuth JWT minted by
//! `POST /api/auth/desktop-token`) lives in the Windows Credential Manager.
//! The embedded shell reads it via `get_token` to authenticate every API call;
//! the login handoff writes it via `save_token`.

const SERVICE: &str = "drivecord-desktop";
const ACCOUNT: &str = "session-token";

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())
}

pub fn read() -> Option<String> {
    match entry().ok()?.get_password() {
        Ok(t) => Some(t),
        Err(_) => None,
    }
}

#[tauri::command]
pub fn save_token(token: String) -> Result<(), String> {
    if token.is_empty() {
        return Err("token vide".into());
    }
    entry()?.set_password(&token).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_token() -> Option<String> {
    read()
}

#[tauri::command]
pub fn clear_token() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
