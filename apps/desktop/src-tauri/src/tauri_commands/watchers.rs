// File and command watcher commands
// - start_file_watcher
// - stop_file_watcher
// - start_command_watcher
// - stop_command_watcher

use std::path::PathBuf;
use tauri::{AppHandle, State};
use crate::tauri_commands::types::AppState;
use crate::watcher::{FileWatcher, CommandWatcher};

#[tauri::command]
pub async fn start_file_watcher(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    messages_dir: Option<String>,
    agent_id: Option<String>,
) -> Result<String, String> {
    let mut watcher_guard = state.file_watcher.lock().await;

    if watcher_guard.is_some() {
        return Err("File watcher is already running".to_string());
    }

    // Default to ~/.agentmux/shared/messages
    let dir = if let Some(dir) = messages_dir {
        PathBuf::from(dir)
    } else {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map_err(|_| "Could not determine home directory".to_string())?;
        PathBuf::from(home).join(".agentmux/shared/messages")
    };

    let mut watcher = FileWatcher::new(dir.clone(), app_handle);

    if let Some(id) = agent_id {
        watcher.set_agent_id(id);
    }

    watcher.start()?;

    *watcher_guard = Some(watcher);

    Ok(format!("File watcher started: {}", dir.display()))
}

#[tauri::command]
pub async fn stop_file_watcher(state: State<'_, AppState>) -> Result<String, String> {
    let mut watcher_guard = state.file_watcher.lock().await;

    if let Some(mut watcher) = watcher_guard.take() {
        watcher.stop();
        Ok("File watcher stopped".to_string())
    } else {
        Err("File watcher is not running".to_string())
    }
}

#[tauri::command]
pub async fn start_command_watcher(
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let mut watcher_guard = state.command_watcher.lock().await;

    if watcher_guard.is_some() {
        return Err("Command watcher is already running".to_string());
    }

    // Default to ~/.agentmux/desktop/commands
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "Could not determine home directory".to_string())?;
    let commands_dir = PathBuf::from(home).join(".agentmux/desktop/commands");

    let mut watcher = CommandWatcher::new(commands_dir.clone(), app_handle);
    watcher.start()?;

    *watcher_guard = Some(watcher);

    Ok(format!("Command watcher started: {}", commands_dir.display()))
}

#[tauri::command]
pub async fn stop_command_watcher(state: State<'_, AppState>) -> Result<String, String> {
    let mut watcher_guard = state.command_watcher.lock().await;

    if let Some(mut watcher) = watcher_guard.take() {
        watcher.stop();
        Ok("Command watcher stopped".to_string())
    } else {
        Err("Command watcher is not running".to_string())
    }
}

#[cfg(test)]
mod tests {
    // Tests for watcher commands require Tauri runtime
    // Covered by integration tests
}
