use crate::state::AppState;

/// Get the auth key for backend communication.
///
/// Replaces: ipcMain.on("get-auth-key") in emain/authkey.ts
///
/// In Electron, the auth key was injected into HTTP requests via
/// session.webRequest.onBeforeSendHeaders(). Tauri doesn't support
/// session-level request interception, so the frontend must include
/// the auth key in requests explicitly (via X-AuthKey header or
/// query parameter).
#[tauri::command]
pub fn get_auth_key(state: tauri::State<'_, AppState>) -> String {
    let key = state.auth_key.lock().unwrap().clone();
    tracing::debug!("Frontend requested auth key: {}", &key[..8]);
    key
}
