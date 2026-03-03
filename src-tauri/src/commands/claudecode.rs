use crate::state::AppState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuthStatus {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

/// Legacy stub — auth is now handled by `claude auth login` via the shell controller.
/// Kept as registered Tauri command for backward compatibility.
#[tauri::command]
pub async fn open_claude_code_auth() -> Result<(), String> {
    tracing::warn!("open_claude_code_auth called — this is a legacy stub. Auth is now handled by the CLI via `claude auth login`.");
    Ok(())
}

/// Legacy stub — auth status is now checked via `check_cli_auth_status` in providers.rs.
#[tauri::command]
pub async fn get_claude_code_auth(_state: tauri::State<'_, AppState>) -> Result<AuthStatus, String> {
    Ok(AuthStatus {
        connected: false,
        email: None,
        expires_at: None,
    })
}

/// Legacy stub — disconnect is handled via `clear_provider_auth` in providers.rs.
#[tauri::command]
pub async fn disconnect_claude_code(_state: tauri::State<'_, AppState>) -> Result<(), String> {
    Ok(())
}
